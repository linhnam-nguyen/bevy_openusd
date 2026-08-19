//! Working semantic query service backed by an in-memory Turso database.
//!
//! Semantic rows remain renderer-neutral. The viewport bridge adapts their
//! prim paths through `SceneAnchorIndex` when publishing search results.

mod diff;
mod query;
mod state;
mod store;
mod types;
mod worker;

use std::collections::{HashMap, HashSet};

use bevy::prelude::World;
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::{LiveRevision, LiveStage, PendingStageChanges};
use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use crate::project::blob_store::FilesystemBlobStore;
use crate::project::ghost_cache::{attach_render_blobs, attach_render_blobs_for_entities};
use crate::project::recovery::RecoverySettings;
use crate::project::runtime_delivery::{build_runtime_delivery, into_delivery_parts};
use crate::viewport::api::RenderServerInterface;

pub(crate) use diff::SemanticDiffState;
pub(crate) use query::{GroupField, SemanticFilter, SemanticQuery, SemanticQueryResult};
pub(crate) use state::SemanticSyncState;
pub(crate) use types::{SemanticIncrementalUpdate, SemanticResponse};
pub(crate) use worker::SemanticWorkingStore;

#[derive(Debug)]
pub(crate) enum SubtreeUpdateError {
    UnnormalizableRoot(String),
    EntityKeyCollision(String),
    ExtractionFailed(anyhow::Error),
}

impl std::fmt::Display for SubtreeUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnnormalizableRoot(msg) => write!(f, "unnormalizable root: {msg}"),
            Self::EntityKeyCollision(msg) => write!(f, "EntityKey collision: {msg}"),
            Self::ExtractionFailed(err) => write!(f, "subtree extraction failed: {err:#}"),
        }
    }
}

impl std::error::Error for SubtreeUpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ExtractionFailed(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for SubtreeUpdateError {
    fn from(err: anyhow::Error) -> Self {
        Self::ExtractionFailed(err)
    }
}

impl SubtreeUpdateError {
    pub(crate) fn fallback_reason(&self) -> &'static str {
        match self {
            Self::UnnormalizableRoot(_) => "unnormalizable_root",
            Self::EntityKeyCollision(_) => "semantic_entity_key_collision",
            Self::ExtractionFailed(_) => "subtree_delta_extraction_failed",
        }
    }
}

/// Synchronize the semantic working store from the retained live-stage batch.
///
/// This is an exclusive system because `LiveStage` is a non-send resource and
/// extraction must borrow its OpenUSD stage while the resulting ECS resource
/// state is updated. It runs after `LiveStagePlugin` has drained the batch.
pub(crate) fn synchronize_live_stage(world: &mut World) {
    let Some((session_id, live_revision, pending_batch, previous_snapshot, previous_session)) =
        (|| {
            let live = world.get_non_send::<LiveStage>()?;
            let pending_batch = world.resource::<PendingStageChanges>().batch().cloned();
            let state = world.resource::<SemanticSyncState>();
            Some((
                live.session_id(),
                live.current_revision(),
                pending_batch,
                state.snapshot.clone(),
                state.session_id,
            ))
        })()
    else {
        return;
    };

    let root_count = pending_batch
        .as_ref()
        .map(|b| {
            if b.has_resync() {
                b.resync_roots().len()
            } else {
                0
            }
        })
        .unwrap_or(0);

    let previous_snapshot = (previous_session == Some(session_id))
        .then_some(previous_snapshot)
        .flatten();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "viewport-working".to_owned(),
        live_revision: live_revision.0,
    };

    let update = {
        let live = world
            .get_non_send::<LiveStage>()
            .expect("live stage exists");
        match previous_snapshot {
            None => match extractor.extract(&live.stage, source) {
                Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                Err(error) => {
                    bevy::log::error!("[semantic-sync] initial snapshot failed: {error:#}");
                    return;
                }
            },
            Some(previous_snapshot) => {
                let Some(batch) = pending_batch else {
                    return;
                };
                let previous_revision = if previous_session == Some(session_id) {
                    world
                        .resource::<SemanticSyncState>()
                        .revision
                        .unwrap_or_default()
                } else {
                    LiveRevision::default()
                };
                if batch.revision <= previous_revision {
                    return;
                }
                if batch.has_resync() {
                    let all_resynced: Vec<&str> = batch
                        .changes
                        .iter()
                        .flat_map(|c| c.resynced.iter().map(String::as_str))
                        .collect();
                    let mut unnormalizable = false;
                    for r in &all_resynced {
                        if let Err(err) = usd_bevy::validate_prim_path(r) {
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = "unnormalizable_root",
                                root_count = all_resynced.len(),
                                live_revision = live_revision.0,
                                "[semantic-sync] root '{r}' cannot represent a safe OpenUSD prim path: {err:#}; falling back to full snapshot rebuild"
                            );
                            unnormalizable = true;
                            break;
                        }
                    }

                    if unnormalizable {
                        match extractor.extract(&live.stage, source) {
                            Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                            Err(err) => {
                                bevy::log::error!(
                                    "[semantic-sync] full snapshot fallback failed: {err:#}"
                                );
                                return;
                            }
                        }
                    } else {
                        let roots = batch.resync_roots();
                        if roots.contains(&"/".to_string()) || roots.is_empty() {
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = "root_is_stage_root_or_empty",
                                root_count = roots.len(),
                                live_revision = live_revision.0,
                                "[semantic-sync] stage root '/' or empty roots in batch; falling back to full snapshot rebuild"
                            );
                            match extractor.extract(&live.stage, source) {
                                Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                                Err(error) => {
                                    bevy::log::error!(
                                        "[semantic-sync] resync full rebuild failed: {error:#}"
                                    );
                                    return;
                                }
                            }
                        } else {
                            match resync_subtree_update(
                                &live.stage,
                                &extractor,
                                previous_snapshot.clone(),
                                &batch,
                                source.clone(),
                            ) {
                                Ok(update) => SemanticSyncAction::Delta(update),
                                Err(err) => {
                                    let reason = err.fallback_reason();
                                    bevy::log::warn!(
                                        target: "semantic_sync",
                                        resync_fallback_reason = reason,
                                        root_count = roots.len(),
                                        live_revision = live_revision.0,
                                        "[semantic-sync] subtree delta extraction failed: {err:#}; falling back to full snapshot rebuild"
                                    );
                                    match extractor.extract(&live.stage, source) {
                                        Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                                        Err(fallback_err) => {
                                            bevy::log::error!(
                                                "[semantic-sync] full snapshot fallback failed: {fallback_err:#}"
                                            );
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    match changed_info_update(
                        &live.stage,
                        &extractor,
                        previous_snapshot,
                        &batch,
                        source.clone(),
                    ) {
                        Ok(update) => SemanticSyncAction::Delta(update),
                        Err(err) => {
                            let reason = err.fallback_reason();
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = reason,
                                root_count = 0usize,
                                live_revision = live_revision.0,
                                "[semantic-sync] changed-info update failed: {err:#}; falling back to full snapshot rebuild"
                            );
                            match extractor.extract(&live.stage, source) {
                                Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                                Err(fallback_err) => {
                                    bevy::log::error!(
                                        "[semantic-sync] full snapshot fallback failed: {fallback_err:#}"
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    let mut update = update;
    attach_render_blobs_to_action(world, &mut update, live_revision, root_count);

    let request_id = format!("semantic-sync-{}", live_revision.0);
    let submitted = match update {
        SemanticSyncAction::Replace(snapshot) => {
            publish_runtime_delivery(world, &snapshot);
            let submitted = world
                .resource::<SemanticWorkingStore>()
                .submit_snapshot(request_id, snapshot.clone());
            if submitted {
                world.resource_mut::<SemanticSyncState>().snapshot = Some(snapshot.clone());
                if let Some(mut diff_state) = world.get_resource_mut::<SemanticDiffState>() {
                    diff_state.update_working(session_id, snapshot);
                }
            }
            submitted
        }
        SemanticSyncAction::Delta(update) => {
            let snapshot = update.snapshot.clone();
            publish_runtime_delivery(world, &snapshot);
            let submitted = world
                .resource::<SemanticWorkingStore>()
                .submit_delta(request_id, update.request);
            if submitted {
                world.resource_mut::<SemanticSyncState>().snapshot = Some(snapshot.clone());
                if let Some(mut diff_state) = world.get_resource_mut::<SemanticDiffState>() {
                    diff_state.update_working(session_id, snapshot);
                }
            }
            submitted
        }
    };
    if submitted {
        let mut state = world.resource_mut::<SemanticSyncState>();
        state.session_id = Some(session_id);
        state.revision = Some(live_revision);
    } else {
        bevy::log::warn!("[semantic-sync] worker channel is unavailable");
    }
}

fn publish_runtime_delivery(world: &World, snapshot: &SemanticSnapshot) {
    let Some(interface) = world
        .get_resource::<RenderServerInterface>()
        .map(RenderServerInterface::shared)
    else {
        // The local/native viewer does not install the WebRTC delivery bus.
        return;
    };
    let Some(settings) = world.get_resource::<RecoverySettings>() else {
        interface.clear_runtime_delivery();
        return;
    };
    let store = match FilesystemBlobStore::new(
        settings
            .project_root
            .join(crate::project::blob_store::OBJECTS_DIRECTORY),
    ) {
        Ok(store) => store,
        Err(error) => {
            interface.clear_runtime_delivery();
            bevy::log::error!("[runtime-delivery] cannot create BlobStore: {error:#}");
            return;
        }
    };
    let bundle = match build_runtime_delivery(
        &store,
        snapshot,
        viewport_protocol::RuntimeProfile::NativeMedium,
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            interface.clear_runtime_delivery();
            bevy::log::warn!("[runtime-delivery] bundle publication skipped: {error:#}");
            return;
        }
    };
    let (manifest, blobs) = into_delivery_parts(bundle);
    if let Err(error) = interface.publish_runtime_delivery(manifest, blobs) {
        interface.clear_runtime_delivery();
        bevy::log::warn!("[runtime-delivery] bundle publication rejected: {error:?}");
    }
}

fn attach_render_blobs_to_action(
    world: &mut World,
    action: &mut SemanticSyncAction,
    live_revision: LiveRevision,
    root_count: usize,
) {
    match action {
        SemanticSyncAction::Replace(snapshot) => attach_render_blobs(world, snapshot),
        SemanticSyncAction::Delta(update) => {
            let Some(map) = world.get_resource::<usd_bevy::PrimEntities>() else {
                bevy::log::warn!(
                    target: "ghost_cache",
                    resync_fallback_reason = "missing_prim_entities_index",
                    root_count = root_count,
                    live_revision = live_revision.0,
                    "[ghost-cache] PrimEntities resource missing from world; falling back to full attach_render_blobs"
                );
                attach_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return;
            };

            // Partial index corruption: PrimEntities exists, but an affected geometry prim has no index mapping
            let has_missing_mapping = update.request.upserts.iter().any(|entity| {
                entity
                    .geometry
                    .as_ref()
                    .map_or(false, |g| g.render_blob.is_none())
                    && map.entity(&entity.prim_path).is_none()
            });

            if has_missing_mapping {
                bevy::log::warn!(
                    target: "ghost_cache",
                    resync_fallback_reason = "partial_prim_entities_index_corruption",
                    root_count = root_count,
                    live_revision = live_revision.0,
                    "[ghost-cache] affected geometry entity missing from PrimEntities index; falling back to full attach_render_blobs"
                );
                attach_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return;
            }

            // Enrich only affected upserted semantic entities
            attach_render_blobs_for_entities(world, &mut update.request.upserts);
            // Copy enriched upserts back into update.snapshot.entities
            for upsert in &update.request.upserts {
                if let Some(entity) = update.snapshot.entities.get_mut(&upsert.key) {
                    entity.geometry = upsert.geometry.clone();
                }
            }
        }
    }
}

enum SemanticSyncAction {
    Replace(SemanticSnapshot),
    Delta(SemanticDelta),
}

struct SemanticDelta {
    request: SemanticIncrementalUpdate,
    snapshot: SemanticSnapshot,
}

/// Derive a scoped semantic delta for subtree resync notices.
fn resync_subtree_update(
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

fn changed_info_update(
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

#[cfg(test)]
mod tests;
