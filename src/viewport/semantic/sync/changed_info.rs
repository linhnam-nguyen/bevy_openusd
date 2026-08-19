use std::collections::{HashMap, HashSet};

use openusd::usd::{PrimPredicate, Stage};
use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotSource};
use usd_semantic::SemanticExtractor;

use super::super::types::SemanticIncrementalUpdate;
use super::action::{SemanticDelta, SubtreeUpdateError};

pub(in crate::viewport::semantic) fn changed_info_update(
    stage: &Stage,
    extractor: &SemanticExtractor,
    previous_snapshot: SemanticSnapshot,
    batch: &usd_bevy::StageChangeBatch,
    source: SnapshotSource,
) -> Result<SemanticDelta, SubtreeUpdateError> {
    let mut affected_paths = HashSet::new();
    for change in &batch.changes {
        for path in &change.changed_info {
            let prim = usd_bevy::prim_of(path);
            affected_paths.insert(usd_bevy::normalize_prim_path(prim));
        }
    }

    let mut available_paths = HashSet::new();
    stage
        .traverse(PrimPredicate::DEFAULT, |path| {
            available_paths.insert(path.as_str().to_owned());
        })
        .map_err(SubtreeUpdateError::ExtractionFailed)?;

    // 1. Capture old affected keys and paths from previous_snapshot before mutation
    let mut old_affected_keys = HashSet::new();
    let mut old_affected_paths = HashSet::new();
    for (key, entity) in &previous_snapshot.entities {
        if affected_paths.contains(&entity.prim_path) {
            old_affected_keys.insert(key.clone());
            old_affected_paths.insert(entity.prim_path.clone());
        }
    }

    let mut working_entities = previous_snapshot.entities;
    for key in &old_affected_keys {
        working_entities.remove(key);
    }

    // 2. Extract current affected entities and check for internal duplicates
    let mut current_entities: HashMap<EntityKey, EntitySnapshot> = HashMap::new();
    let mut path_by_key: HashMap<EntityKey, String> = HashMap::new();
    let mut sorted_paths: Vec<_> = affected_paths.into_iter().collect();
    sorted_paths.sort();

    for path in &sorted_paths {
        if available_paths.contains(path) {
            let usd_path = openusd::sdf::path(path).map_err(|e| {
                SubtreeUpdateError::ExtractionFailed(anyhow::anyhow!("invalid path {path}: {e}"))
            })?;
            let entity = extractor
                .extract_entity(stage, &usd_path)
                .map_err(SubtreeUpdateError::ExtractionFailed)?;
            if let Some(existing_path) =
                path_by_key.insert(entity.key.clone(), entity.prim_path.clone())
            {
                return Err(SubtreeUpdateError::EntityKeyCollision(format!(
                    "EntityKey collision among extracted prims: '{}' and '{}' both generated key {:?}",
                    existing_path, entity.prim_path, entity.key
                )));
            }
            current_entities.insert(entity.key.clone(), entity);
        }
    }

    // 3. Reject collisions with unaffected entities in working_entities
    for (key, entity) in &current_entities {
        if let Some(existing) = working_entities.get(key) {
            return Err(SubtreeUpdateError::EntityKeyCollision(format!(
                "EntityKey collision: extracted entity at '{}' collided with unaffected entity at '{}' (key: {:?})",
                entity.prim_path, existing.prim_path, key
            )));
        }
    }

    // 4. Compute upserts and removed_paths
    let mut upserts = Vec::new();
    let mut current_paths_set = HashSet::new();
    for (key, entity) in current_entities {
        current_paths_set.insert(entity.prim_path.clone());
        working_entities.insert(key, entity.clone());
        upserts.push(entity);
    }
    upserts.sort_by(|a, b| a.prim_path.cmp(&b.prim_path));

    let mut removed_paths: Vec<String> = old_affected_paths
        .into_iter()
        .filter(|path| !current_paths_set.contains(path))
        .collect();
    removed_paths.sort();

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
