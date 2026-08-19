use std::collections::{HashMap, HashSet};

use openusd::usd::Stage;
use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotSource};
use usd_semantic::SemanticExtractor;

use super::super::types::SemanticIncrementalUpdate;
use super::action::{SemanticDelta, SubtreeUpdateError};

/// Derive a scoped semantic delta for subtree resync notices.
pub(crate) fn resync_subtree_update(
    stage: &Stage,
    extractor: &SemanticExtractor,
    previous_snapshot: SemanticSnapshot,
    batch: &usd_bevy::StageChangeBatch,
    source: SnapshotSource,
) -> Result<SemanticDelta, SubtreeUpdateError> {
    let roots = batch.resync_roots();
    if roots.is_empty() || roots.contains(&"/".to_string()) {
        return Err(SubtreeUpdateError::UnnormalizableRoot(
            "Stage root resync cannot be processed as a subtree delta".to_owned(),
        ));
    }
    for root in &roots {
        if let Err(e) = usd_bevy::validate_prim_path(root) {
            return Err(SubtreeUpdateError::UnnormalizableRoot(format!(
                "{root}: {e}"
            )));
        }
    }

    // 1. Capture old affected entities under resync roots from previous_snapshot
    let mut old_affected_keys = HashSet::new();
    let mut old_affected_paths = HashSet::new();
    for (key, entity) in &previous_snapshot.entities {
        if roots
            .iter()
            .any(|root| usd_bevy::is_descendant_or_self(root, &entity.prim_path))
        {
            old_affected_keys.insert(key.clone());
            old_affected_paths.insert(entity.prim_path.clone());
        }
    }

    // 2. Capture old entities for unshaded changed_info paths from previous_snapshot by prim_path
    let unshaded = batch.unshaded_changed_info();
    let mut unshaded_paths_to_extract = HashSet::new();
    for info_path in unshaded {
        let prim = usd_bevy::prim_of(&info_path);
        let norm = usd_bevy::normalize_prim_path(prim);
        if !roots
            .iter()
            .any(|root| usd_bevy::is_descendant_or_self(root, &norm))
        {
            unshaded_paths_to_extract.insert(norm.clone());
            for (key, entity) in &previous_snapshot.entities {
                if entity.prim_path == norm {
                    old_affected_keys.insert(key.clone());
                    old_affected_paths.insert(entity.prim_path.clone());
                }
            }
        }
    }

    // 3. Collect current stage subtree prim paths for all minimal roots
    let mut prim_paths_to_extract = HashSet::new();
    for root in &roots {
        let paths = usd_bevy::collect_stage_subtree_paths(stage, root)
            .map_err(SubtreeUpdateError::ExtractionFailed)?;
        prim_paths_to_extract.extend(paths);
    }

    // 4. Merge unshaded changed_info paths (avoiding duplicate extractions)
    for path in unshaded_paths_to_extract {
        prim_paths_to_extract.insert(path);
    }

    // 5. Extract current affected entities with collision-safe checking
    let mut sorted_paths: Vec<_> = prim_paths_to_extract.into_iter().collect();
    sorted_paths.sort();

    let mut current_entities: HashMap<EntityKey, EntitySnapshot> = HashMap::new();
    let mut path_by_key: HashMap<EntityKey, String> = HashMap::new();

    for path_str in sorted_paths {
        let usd_path = openusd::sdf::path(&path_str).map_err(|e| {
            SubtreeUpdateError::ExtractionFailed(anyhow::anyhow!("invalid path {path_str}: {e}"))
        })?;
        if stage
            .prim(usd_path.clone())
            .is_valid()
            .map_err(SubtreeUpdateError::ExtractionFailed)?
        {
            let entity = extractor
                .extract_entity(stage, &usd_path)
                .map_err(SubtreeUpdateError::ExtractionFailed)?;
            // Current vs current duplicate EntityKey check
            if let Some(existing_path) =
                path_by_key.insert(entity.key.clone(), entity.prim_path.clone())
            {
                return Err(SubtreeUpdateError::EntityKeyCollision(format!(
                    "Duplicate EntityKey collision among extracted prims: '{}' and '{}' both generated key {:?}",
                    existing_path, entity.prim_path, entity.key
                )));
            }
            current_entities.insert(entity.key.clone(), entity);
        }
    }

    // Remove old affected entities from working map
    let mut working_entities = previous_snapshot.entities;
    for key in &old_affected_keys {
        working_entities.remove(key);
    }

    // Current vs unaffected EntityKey collision check
    for (key, entity) in &current_entities {
        if let Some(existing) = working_entities.get(key) {
            return Err(SubtreeUpdateError::EntityKeyCollision(format!(
                "EntityKey collision: extracted entity at '{}' collided with unaffected entity at '{}' (key: {:?})",
                entity.prim_path, existing.prim_path, key
            )));
        }
    }

    // 6. Compute removed_paths (old affected paths - current affected paths)
    let current_paths_set: HashSet<String> = current_entities
        .values()
        .map(|e| e.prim_path.clone())
        .collect();
    let mut removed_paths: Vec<String> = old_affected_paths
        .into_iter()
        .filter(|path| !current_paths_set.contains(path))
        .collect();
    removed_paths.sort();

    // 7. Compute upserts (current subtree + unshaded changed_info)
    let mut upserts = Vec::new();
    for (key, entity) in current_entities {
        working_entities.insert(key, entity.clone());
        upserts.push(entity);
    }
    upserts.sort_by(|a, b| a.prim_path.cmp(&b.prim_path));

    // 8. Rebuild authoritative snapshot
    let snapshot = extractor.snapshot_from_entities(source, working_entities);
    let request = SemanticIncrementalUpdate {
        snapshot_id: snapshot.snapshot_id.clone(),
        source: snapshot.source.clone(),
        config_hash: snapshot.config_hash,
        upserts,
        removed_paths,
    };
    Ok(SemanticDelta { request, snapshot })
}
