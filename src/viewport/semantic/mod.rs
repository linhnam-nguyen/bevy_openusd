//! Working semantic query service backed by an in-memory Turso database.
//!
//! Semantic rows remain renderer-neutral. The viewport bridge adapts their
//! prim paths through `SceneAnchorIndex` when publishing search results.

mod query;
mod store;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, mpsc};

use bevy::prelude::{Resource, World};
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::{LiveRevision, LiveStage, PendingStageChanges};
use usd_diff::{DiffSummary, StageDiff};
use usd_model::{EntitySnapshot, HashDigest, SemanticSnapshot, SnapshotId, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use crate::project::blob_store::FilesystemBlobStore;
use crate::project::ghost_cache::attach_render_blobs;
use crate::project::recovery::RecoverySettings;
use crate::project::runtime_delivery::{build_runtime_delivery, into_delivery_parts};
use crate::viewport::api::RenderServerInterface;

pub(crate) use query::{GroupField, SemanticFilter, SemanticQuery, SemanticQueryResult};

use store::SemanticDatabase;

#[derive(Debug)]
pub(crate) struct SemanticIncrementalUpdate {
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) source: SnapshotSource,
    pub(crate) config_hash: HashDigest,
    pub(crate) upserts: Vec<EntitySnapshot>,
    pub(crate) removed_paths: Vec<String>,
}

#[derive(Debug)]
enum SemanticCommand {
    ReplaceSnapshot {
        request_id: String,
        snapshot: SemanticSnapshot,
    },
    ApplyDelta {
        request_id: String,
        update: SemanticIncrementalUpdate,
    },
    Query {
        request_id: String,
        query: SemanticQuery,
    },
}

#[derive(Debug)]
pub(crate) enum SemanticResponse {
    SnapshotLoaded {
        request_id: String,
        entity_count: u32,
    },
    DeltaApplied {
        request_id: String,
        upserted: u32,
        removed: u32,
    },
    QueryResult {
        request_id: String,
        result: SemanticQueryResult,
    },
    Failed {
        request_id: String,
        operation: &'static str,
        error: String,
    },
}

/// The Bevy-facing channel endpoint for the dedicated semantic worker.
#[derive(Resource, Debug)]
pub(crate) struct SemanticWorkingStore {
    commands: mpsc::Sender<SemanticCommand>,
    responses: Mutex<mpsc::Receiver<SemanticResponse>>,
}

impl Default for SemanticWorkingStore {
    fn default() -> Self {
        let (commands, pending_commands) = mpsc::channel();
        let (responses, pending_responses) = mpsc::channel();
        std::thread::Builder::new()
            .name("usdview-semantic-worker".to_owned())
            .spawn(move || semantic_worker(pending_commands, responses))
            .expect("semantic worker should start");
        Self {
            commands,
            responses: Mutex::new(pending_responses),
        }
    }
}

impl SemanticWorkingStore {
    pub(crate) fn submit_snapshot(
        &self,
        request_id: impl Into<String>,
        snapshot: SemanticSnapshot,
    ) -> bool {
        self.commands
            .send(SemanticCommand::ReplaceSnapshot {
                request_id: request_id.into(),
                snapshot,
            })
            .is_ok()
    }

    pub(crate) fn submit_query(&self, request_id: impl Into<String>, query: SemanticQuery) -> bool {
        self.commands
            .send(SemanticCommand::Query {
                request_id: request_id.into(),
                query,
            })
            .is_ok()
    }

    pub(crate) fn submit_delta(
        &self,
        request_id: impl Into<String>,
        update: SemanticIncrementalUpdate,
    ) -> bool {
        self.commands
            .send(SemanticCommand::ApplyDelta {
                request_id: request_id.into(),
                update,
            })
            .is_ok()
    }

    pub(crate) fn drain_responses(&self) -> Vec<SemanticResponse> {
        let Ok(responses) = self.responses.lock() else {
            return Vec::new();
        };
        responses.try_iter().collect()
    }
}

fn semantic_worker(
    pending_commands: mpsc::Receiver<SemanticCommand>,
    responses: mpsc::Sender<SemanticResponse>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("semantic worker runtime should build");
    let mut database = runtime.block_on(SemanticDatabase::open()).ok();
    let mut buffered_command = None;

    loop {
        let Some(command) = buffered_command
            .take()
            .or_else(|| pending_commands.recv().ok())
        else {
            break;
        };

        // Preserve the old search-worker behavior: a burst of consecutive
        // query requests can be coalesced, while snapshot/delta commands stay
        // ordered and act as barriers for the query stream.
        let command = match command {
            SemanticCommand::Query {
                mut request_id,
                mut query,
            } => {
                while let Ok(next) = pending_commands.try_recv() {
                    match next {
                        SemanticCommand::Query {
                            request_id: newer_request_id,
                            query: newer_query,
                        } => {
                            request_id = newer_request_id;
                            query = newer_query;
                        }
                        other => {
                            buffered_command = Some(other);
                            break;
                        }
                    }
                }
                SemanticCommand::Query { request_id, query }
            }
            other => other,
        };

        let (request_id, result, operation) = match command {
            SemanticCommand::ReplaceSnapshot {
                request_id,
                snapshot,
            } => {
                let result = database.as_mut().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.replace_snapshot(&snapshot))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|count| SemanticResponse::SnapshotLoaded {
                        request_id: String::new(),
                        entity_count: count,
                    }),
                    "snapshot load",
                )
            }
            SemanticCommand::ApplyDelta { request_id, update } => {
                let result = database.as_mut().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.apply_delta(&update))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|(upserted, removed)| SemanticResponse::DeltaApplied {
                        request_id: String::new(),
                        upserted,
                        removed,
                    }),
                    "semantic delta",
                )
            }
            SemanticCommand::Query { request_id, query } => {
                let result = database.as_ref().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.query(&query))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|result| SemanticResponse::QueryResult {
                        request_id: String::new(),
                        result,
                    }),
                    "query",
                )
            }
        };

        let response = match result {
            Ok(mut response) => {
                match &mut response {
                    SemanticResponse::SnapshotLoaded {
                        request_id: response_id,
                        ..
                    }
                    | SemanticResponse::DeltaApplied {
                        request_id: response_id,
                        ..
                    }
                    | SemanticResponse::QueryResult {
                        request_id: response_id,
                        ..
                    } => *response_id = request_id,
                    SemanticResponse::Failed { .. } => {}
                }
                response
            }
            Err(error) => SemanticResponse::Failed {
                request_id,
                operation,
                error,
            },
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}

/// Local authoritative semantic state used to derive the next incremental
/// update from the same live-stage revision consumed by Bevy projection.
#[derive(Resource, Default)]
pub(crate) struct SemanticSyncState {
    snapshot: Option<SemanticSnapshot>,
    session_id: Option<u64>,
    revision: Option<LiveRevision>,
}

impl SemanticSyncState {
    pub(crate) fn snapshot(&self) -> Option<&SemanticSnapshot> {
        self.snapshot.as_ref()
    }
}

/// Manual working-vs-baseline comparison state for diagnostics.
///
/// The baseline is intentionally an in-memory snapshot. Git-backed baselines
/// are introduced by the later `usd_git` milestone; this resource only makes
/// the current live semantic snapshot observable through `usd_diff`.
#[derive(Resource, Default)]
pub(crate) struct SemanticDiffState {
    baseline: Option<SemanticSnapshot>,
    working: Option<SemanticSnapshot>,
    session_id: Option<u64>,
    diff: Option<StageDiff>,
}

impl SemanticDiffState {
    pub(crate) fn update_working(&mut self, session_id: u64, snapshot: SemanticSnapshot) {
        if self.session_id != Some(session_id) {
            self.baseline = None;
            self.diff = None;
            self.session_id = Some(session_id);
        }
        self.working = Some(snapshot);
        self.recompute();
    }

    pub(crate) fn capture_baseline(&mut self) -> bool {
        let Some(working) = self.working.clone() else {
            return false;
        };
        self.baseline = Some(working);
        self.recompute();
        true
    }

    pub(crate) fn clear_baseline(&mut self) {
        self.baseline = None;
        self.diff = None;
    }

    pub(crate) fn has_working_snapshot(&self) -> bool {
        self.working.is_some()
    }

    pub(crate) fn has_baseline(&self) -> bool {
        self.baseline.is_some()
    }

    pub(crate) fn summary(&self) -> Option<DiffSummary> {
        self.diff.as_ref().map(|diff| diff.summary)
    }

    pub(crate) fn stage_diff(&self) -> Option<&StageDiff> {
        self.diff.as_ref()
    }

    pub(crate) fn baseline_snapshot_id(&self) -> Option<&SnapshotId> {
        self.baseline.as_ref().map(|snapshot| &snapshot.snapshot_id)
    }

    fn recompute(&mut self) {
        self.diff = self
            .baseline
            .as_ref()
            .zip(self.working.as_ref())
            .map(|(baseline, working)| usd_diff::compare(baseline, working));
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
                    let roots = batch.resync_roots();
                    if roots.contains(&"/".to_string()) || roots.is_empty() {
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
                            Err(error) => {
                                bevy::log::warn!(
                                    "[semantic-sync] subtree delta extraction failed: {error:#}; falling back to full snapshot rebuild"
                                );
                                match extractor.extract(&live.stage, source) {
                                    Ok(snapshot) => SemanticSyncAction::Replace(snapshot),
                                    Err(err) => {
                                        bevy::log::error!(
                                            "[semantic-sync] full snapshot fallback failed: {err:#}"
                                        );
                                        return;
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
                        source,
                    ) {
                        Ok(update) => SemanticSyncAction::Delta(update),
                        Err(error) => {
                            bevy::log::error!(
                                "[semantic-sync] changed-info update failed: {error:#}"
                            );
                            return;
                        }
                    }
                }
            }
        }
    };

    let mut update = update;
    attach_render_blobs_to_action(world, &mut update);

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

fn attach_render_blobs_to_action(world: &mut World, action: &mut SemanticSyncAction) {
    match action {
        SemanticSyncAction::Replace(snapshot) => attach_render_blobs(world, snapshot),
        SemanticSyncAction::Delta(update) => {
            attach_render_blobs(world, &mut update.snapshot);
            for upsert in &mut update.request.upserts {
                if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                    *upsert = enriched.clone();
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
) -> anyhow::Result<SemanticDelta> {
    let roots = batch.resync_roots();
    if roots.is_empty() || roots.contains(&"/".to_string()) {
        anyhow::bail!("Stage root resync cannot be processed as a subtree delta");
    }

    // 1. Collect old affected entities from previous snapshot BEFORE mutation
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

    // 2. Collect current stage subtree prim paths for all minimal roots
    let mut current_prim_paths = HashSet::new();
    for root in &roots {
        let paths = usd_bevy::collect_stage_subtree_paths(stage, root)?;
        current_prim_paths.extend(paths);
    }

    // 3. Extract current affected entities
    let mut current_entities = HashMap::new();
    let mut sorted_current_paths: Vec<_> = current_prim_paths.into_iter().collect();
    sorted_current_paths.sort();
    for path_str in sorted_current_paths {
        let usd_path = openusd::sdf::path(&path_str)?;
        let entity = extractor.extract_entity(stage, &usd_path)?;
        current_entities.insert(entity.key.clone(), entity);
    }

    // 4. Merge unshaded changed-info notices outside resync roots
    let unshaded = batch.unshaded_changed_info();
    let mut unshaded_prim_paths = HashSet::new();
    for info_path in unshaded {
        let prim = usd_bevy::prim_of(&info_path);
        let norm = usd_bevy::normalize_prim_path(prim);
        if !roots
            .iter()
            .any(|root| usd_bevy::is_descendant_or_self(root, &norm))
        {
            unshaded_prim_paths.insert(norm);
        }
    }
    let mut sorted_unshaded: Vec<_> = unshaded_prim_paths.into_iter().collect();
    sorted_unshaded.sort();
    for path_str in sorted_unshaded {
        if let Ok(usd_path) = openusd::sdf::path(&path_str) {
            if let Ok(entity) = extractor.extract_entity(stage, &usd_path) {
                old_affected_keys.insert(entity.key.clone());
                old_affected_paths.insert(entity.prim_path.clone());
                current_entities.insert(entity.key.clone(), entity);
            }
        }
    }

    // 5. Remove old affected entries from working map & validate against identity collisions
    let mut working_entities = previous_snapshot.entities;
    for key in &old_affected_keys {
        working_entities.remove(key);
    }

    for (key, entity) in &current_entities {
        if let Some(existing) = working_entities.get(key) {
            anyhow::bail!(
                "EntityKey collision: extracted entity at '{}' collided with unaffected entity at '{}' (key: {:?})",
                entity.prim_path,
                existing.prim_path,
                key
            );
        }
    }

    // 6. Insert upserts and compute removed_paths
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

    // 7. Rebuild authoritative snapshot
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
    mut previous_snapshot: SemanticSnapshot,
    batch: &usd_bevy::StageChangeBatch,
    source: SnapshotSource,
) -> anyhow::Result<SemanticDelta> {
    let mut affected_paths = HashSet::new();
    for change in &batch.changes {
        for path in &change.changed_info {
            affected_paths.insert(path.split('.').next().unwrap_or(path).to_owned());
        }
    }

    let mut available_paths = HashSet::new();
    stage.traverse(PrimPredicate::DEFAULT, |path| {
        available_paths.insert(path.as_str().to_owned());
    })?;

    previous_snapshot
        .entities
        .retain(|_, entity| !affected_paths.contains(&entity.prim_path));
    let mut upserts = Vec::new();
    let mut removed_paths = Vec::new();
    let mut sorted_paths: Vec<_> = affected_paths.into_iter().collect();
    sorted_paths.sort();
    for path in sorted_paths {
        if available_paths.contains(&path) {
            let usd_path = openusd::sdf::path(&path)?;
            let entity = extractor.extract_entity(stage, &usd_path)?;
            previous_snapshot
                .entities
                .insert(entity.key.clone(), entity.clone());
            upserts.push(entity);
        } else {
            removed_paths.push(path);
        }
    }

    let snapshot = extractor.snapshot_from_entities(source, previous_snapshot.entities);
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
mod tests {
    use anyhow::Result;
    use bevy::prelude::World;
    use bevy::prelude::*;
    use openusd::usd::Stage;
    use usd_bevy::{LiveRevision, LiveStage, StageChange, StageChangeBatch};
    use usd_model::{CanonicalValue, EntityKey, HashDigest, SemanticSnapshot, SnapshotSource};
    use usd_semantic::{SemanticConfig, SemanticExtractor};

    use super::{
        SemanticDiffState, SemanticFilter, SemanticIncrementalUpdate, SemanticQuery,
        SemanticResponse, SemanticSyncState, SemanticWorkingStore, changed_info_update,
        synchronize_live_stage,
    };

    fn snapshot() -> Result<SemanticSnapshot> {
        let stage = Stage::open("tests/stages/custom_attrs_extensive.usda")?;
        SemanticExtractor::new(SemanticConfig::default()).extract(
            &stage,
            SnapshotSource::Working {
                session: "semantic-worker-test".to_owned(),
                live_revision: 1,
            },
        )
    }

    fn response(store: &SemanticWorkingStore) -> SemanticResponse {
        for _ in 0..200 {
            if let Some(response) = store.drain_responses().into_iter().next() {
                return response;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("semantic worker did not respond")
    }

    #[test]
    fn full_snapshot_bulk_load_supports_type_and_property_queries() -> Result<()> {
        let store = SemanticWorkingStore::default();
        let snapshot = snapshot()?;
        let expected_entities = snapshot.entities.len() as u32;
        assert!(store.submit_snapshot("load-1", snapshot));
        assert!(matches!(
            response(&store),
            SemanticResponse::SnapshotLoaded {
                request_id,
                entity_count
            } if request_id == "load-1" && entity_count == expected_entities
        ));

        assert!(store.submit_query(
            "query-type",
            SemanticQuery {
                filters: vec![SemanticFilter::TypeEquals("Cube".to_owned())],
                ..Default::default()
            },
        ));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected query result")
        };
        assert_eq!(result.total, 1);
        assert_eq!(result.rows[0].entity_key, EntityKey::from("/World/Robot"));

        assert!(store.submit_query(
            "query-property",
            SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "userProperties:name".to_owned(),
                    value: "cart_01".to_owned(),
                }],
                ..Default::default()
            },
        ));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected property query result")
        };
        assert_eq!(result.total, 1);
        assert_eq!(result.rows[0].prim_path, "/World/Robot");
        Ok(())
    }

    #[test]
    fn schema_query_supports_grouping_and_pagination() -> Result<()> {
        let store = SemanticWorkingStore::default();
        assert!(store.submit_snapshot("load-2", snapshot()?));
        let _ = response(&store);
        assert!(store.submit_query(
            "query-group",
            SemanticQuery {
                group_by: vec![super::GroupField::TypeName],
                limit: 1,
                ..Default::default()
            },
        ));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected grouped query result")
        };
        assert!(result.total >= 2);
        assert_eq!(result.rows.len(), 1);
        assert!(!result.groups.is_empty());
        assert!(result.has_more);
        Ok(())
    }

    #[test]
    fn changed_info_delta_updates_only_the_affected_semantic_entity() -> Result<()> {
        let store = SemanticWorkingStore::default();
        let snapshot = snapshot()?;
        let entity_count = snapshot.entities.len() as u32;
        assert!(store.submit_snapshot("load-delta", snapshot.clone()));
        let _ = response(&store);

        let mut robot = snapshot
            .entities
            .get(&EntityKey::from("/World/Robot"))
            .cloned()
            .expect("fixture robot entity");
        let property = robot
            .properties
            .iter_mut()
            .find(|property| property.name == "userProperties:name")
            .expect("fixture robot property");
        property.value = CanonicalValue::Text("cart_02".to_owned());

        assert!(store.submit_delta(
            "delta-1",
            SemanticIncrementalUpdate {
                snapshot_id: snapshot.snapshot_id.clone(),
                source: SnapshotSource::Working {
                    session: "semantic-worker-test".to_owned(),
                    live_revision: 2,
                },
                config_hash: snapshot.config_hash,
                upserts: vec![robot],
                removed_paths: Vec::new(),
            },
        ));
        assert!(matches!(
            response(&store),
            SemanticResponse::DeltaApplied {
                request_id,
                upserted: 1,
                removed: 0,
            } if request_id == "delta-1"
        ));

        assert!(store.submit_query(
            "query-updated-property",
            SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "userProperties:name".to_owned(),
                    value: "cart_02".to_owned(),
                }],
                ..Default::default()
            },
        ));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected updated property query result")
        };
        assert_eq!(result.total, 1);
        assert_eq!(result.rows[0].prim_path, "/World/Robot");

        assert!(store.submit_query("query-all-after-delta", SemanticQuery::default()));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected full query result")
        };
        assert_eq!(result.total, entity_count);
        Ok(())
    }

    #[test]
    fn resync_full_replace_removes_entities_from_the_working_store() -> Result<()> {
        let store = SemanticWorkingStore::default();
        let initial = snapshot()?;
        let initial_count = initial.entities.len() as u32;
        assert!(store.submit_snapshot("load-resync", initial.clone()));
        let _ = response(&store);

        let mut entities = initial.entities.clone();
        entities.remove(&EntityKey::from("/World/Robot"));
        let rebuilt = SemanticExtractor::new(SemanticConfig::default()).snapshot_from_entities(
            SnapshotSource::Working {
                session: "semantic-worker-test".to_owned(),
                live_revision: 3,
            },
            entities,
        );
        assert!(store.submit_snapshot("resync-1", rebuilt));
        assert!(matches!(
            response(&store),
            SemanticResponse::SnapshotLoaded {
                request_id,
                entity_count
            } if request_id == "resync-1" && entity_count == initial_count - 1
        ));

        assert!(store.submit_query(
            "query-removed-type",
            SemanticQuery {
                filters: vec![SemanticFilter::TypeEquals("Cube".to_owned())],
                ..Default::default()
            },
        ));
        let SemanticResponse::QueryResult { result, .. } = response(&store) else {
            panic!("expected rebuilt query result")
        };
        assert_eq!(result.total, 0);
        Ok(())
    }

    #[test]
    fn changed_info_extraction_replaces_only_the_affected_prim() -> Result<()> {
        let stage = Stage::open("tests/stages/custom_attrs_extensive.usda")?;
        let extractor = SemanticExtractor::new(SemanticConfig::default());
        let before = extractor.extract(
            &stage,
            SnapshotSource::Working {
                session: "semantic-sync-test".to_owned(),
                live_revision: 1,
            },
        )?;
        let batch = StageChangeBatch {
            revision: LiveRevision(2),
            changes: vec![StageChange {
                resynced: Vec::new(),
                changed_info: vec!["/World/Robot.userProperties:name".to_owned()],
            }],
        };

        let delta = changed_info_update(
            &stage,
            &extractor,
            before.clone(),
            &batch,
            SnapshotSource::Working {
                session: "semantic-sync-test".to_owned(),
                live_revision: 2,
            },
        )?;
        assert_eq!(delta.request.upserts.len(), 1);
        assert_eq!(delta.request.upserts[0].prim_path, "/World/Robot");
        assert!(delta.request.removed_paths.is_empty());
        assert_eq!(delta.snapshot.entities.len(), before.entities.len());
        Ok(())
    }

    #[test]
    fn manual_baseline_recomputes_for_working_changes_and_resets_on_reload() -> Result<()> {
        let initial = snapshot()?;
        let mut state = SemanticDiffState::default();
        state.update_working(1, initial.clone());

        assert!(state.capture_baseline());
        let initial_summary = state.summary().expect("baseline and working are present");
        assert_eq!(initial_summary.added, 0);
        assert_eq!(initial_summary.removed, 0);
        assert_eq!(initial_summary.changed, 0);
        assert_eq!(initial_summary.unchanged, initial.entities.len());

        let mut changed = initial;
        let key = changed
            .entities
            .keys()
            .next()
            .cloned()
            .expect("fixture contains semantic entities");
        let entity = changed
            .entities
            .get_mut(&key)
            .expect("entity key came from the snapshot");
        entity.prim_path.push_str("/Moved");
        entity.full_hash = HashDigest::new([0xa5; HashDigest::BYTE_LEN]);
        state.update_working(1, changed);

        let summary = state.summary().expect("baseline and working are present");
        assert_eq!(summary.changed, 1);
        assert_eq!(summary.path, 1);
        assert_eq!(summary.transform, 0);
        assert_eq!(summary.metadata, 0);
        assert_eq!(summary.geometry, 0);

        state.update_working(2, snapshot()?);
        assert!(!state.has_baseline());
        assert_eq!(state.summary(), None);
        Ok(())
    }

    #[test]
    fn a_new_live_stage_session_triggers_a_full_semantic_load() -> Result<()> {
        let mut world = World::new();
        world.insert_resource(SemanticWorkingStore::default());
        world.insert_resource(usd_bevy::PendingStageChanges::default());
        world.insert_resource(SemanticSyncState::default());
        world.insert_non_send(LiveStage::new(Stage::open(
            "tests/stages/custom_attrs_extensive.usda",
        )?));

        synchronize_live_stage(&mut world);
        assert!(matches!(
            response(world.resource::<SemanticWorkingStore>()),
            SemanticResponse::SnapshotLoaded { .. }
        ));

        world.remove_non_send::<LiveStage>();
        world.insert_non_send(LiveStage::new(Stage::open(
            "tests/stages/custom_attrs_extensive.usda",
        )?));
        synchronize_live_stage(&mut world);
        assert!(matches!(
            response(world.resource::<SemanticWorkingStore>()),
            SemanticResponse::SnapshotLoaded { .. }
        ));
        Ok(())
    }

    #[test]
    fn resync_notice_triggers_full_snapshot_replace_in_semantic_store_baseline() -> Result<()> {
        let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
        for group in ["A", "B", "C"] {
            usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
            for i in 0..10 {
                usda.push_str(&format!(
                    "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
                ));
            }
            usda.push_str("    }\n");
        }
        usda.push_str("}\n");

        let stage = usd_bevy::UsdSnippet::new(&usda)
            .open_stage()
            .expect("synthetic wide stage opens");
        let live = LiveStage::new(stage);

        let mut app = App::new();
        app.add_plugins(usd_bevy::LiveStagePlugin);
        app.insert_resource(SemanticWorkingStore::default());
        app.insert_resource(SemanticSyncState::default());
        app.world_mut().insert_non_send(live);
        app.add_systems(PostUpdate, synchronize_live_stage);

        // Initial frame: LiveStagePlugin performs initial project_stage,
        // synchronize_live_stage performs initial full snapshot load (34 prims)
        app.update();
        let resp = response(app.world().resource::<SemanticWorkingStore>());
        let initial_count = match resp {
            SemanticResponse::SnapshotLoaded { entity_count, .. } => entity_count,
            other => panic!("expected initial SnapshotLoaded, got {other:?}"),
        };
        assert_eq!(initial_count, 34);

        // Enqueue resync on /World/B (11 prims affected)
        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .load_payload("/World/B");

        // Next frame: LiveStagePlugin drains the change batch into PendingStageChanges,
        // and synchronize_live_stage consumes it through normal Bevy scheduling.
        app.update();

        let resp = response(app.world().resource::<SemanticWorkingStore>());
        match resp {
            SemanticResponse::DeltaApplied {
                upserted, removed, ..
            } => {
                // Subtree resync extracts and upserts only the 11 prims under /World/B
                assert_eq!(upserted, 11);
                assert_eq!(removed, 0);
            }
            other => panic!("expected DeltaApplied for subtree resync, got {other:?}"),
        }

        // Query Turso to confirm all 34 rows remain present in working store
        let store = app.world().resource::<SemanticWorkingStore>();
        assert!(store.submit_query("verify-all-34", SemanticQuery::default()));
        let SemanticResponse::QueryResult { result, .. } = response(store) else {
            panic!("expected query result")
        };
        assert_eq!(result.total, 34);
        Ok(())
    }

    #[test]
    fn resync_subtree_spawns_and_despawns_semantic_entities() -> Result<()> {
        let stage = Stage::builder()
            .in_memory("semantic-subtree-spawn-despawn.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();
        stage.define_prim("/World/A/Child1").unwrap();
        stage.define_prim("/World/A/Child2").unwrap();
        stage.define_prim("/World/B").unwrap();

        let live = LiveStage::new(stage);

        let mut app = App::new();
        app.add_plugins(usd_bevy::LiveStagePlugin);
        app.insert_resource(SemanticWorkingStore::default());
        app.insert_resource(SemanticSyncState::default());
        app.world_mut().insert_non_send(live);
        app.add_systems(PostUpdate, synchronize_live_stage);

        app.update();
        let resp = response(app.world().resource::<SemanticWorkingStore>());
        assert!(matches!(
            resp,
            SemanticResponse::SnapshotLoaded {
                entity_count: 5,
                ..
            }
        ));

        // Remove Child2, add Child3
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.remove_prim("/World/A/Child2").unwrap();
        live.stage.define_prim("/World/A/Child3").unwrap();
        let _ = live.drain_change_batch();
        live.load_payload("/World/A");

        app.update();

        let resp = response(app.world().resource::<SemanticWorkingStore>());
        match resp {
            SemanticResponse::DeltaApplied {
                upserted, removed, ..
            } => {
                // /World/A, /World/A/Child1, /World/A/Child3 = 3 upserts; /World/A/Child2 = 1 removal
                assert_eq!(upserted, 3);
                assert_eq!(removed, 1);
            }
            other => panic!("expected DeltaApplied, got {other:?}"),
        }

        // Query Turso: total 5 rows (/World, /World/A, /World/A/Child1, /World/A/Child3, /World/B)
        let store = app.world().resource::<SemanticWorkingStore>();
        assert!(store.submit_query("verify-count-5", SemanticQuery::default()));
        let SemanticResponse::QueryResult { result, .. } = response(store) else {
            panic!("expected query result")
        };
        assert_eq!(result.total, 5);
        Ok(())
    }

    #[test]
    fn resync_stage_root_triggers_full_snapshot_replace() -> Result<()> {
        let stage = Stage::builder()
            .in_memory("semantic-root-resync.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();
        stage.define_prim("/World/B").unwrap();

        let live = LiveStage::new(stage);

        let mut app = App::new();
        app.add_plugins(usd_bevy::LiveStagePlugin);
        app.insert_resource(SemanticWorkingStore::default());
        app.insert_resource(SemanticSyncState::default());
        app.world_mut().insert_non_send(live);
        app.add_systems(PostUpdate, synchronize_live_stage);

        app.update();
        let resp = response(app.world().resource::<SemanticWorkingStore>());
        assert!(matches!(
            resp,
            SemanticResponse::SnapshotLoaded {
                entity_count: 3,
                ..
            }
        ));

        // Resync root "/"
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.load_payload("/");

        app.update();

        let resp = response(app.world().resource::<SemanticWorkingStore>());
        match resp {
            SemanticResponse::SnapshotLoaded { entity_count, .. } => {
                assert_eq!(entity_count, 3);
            }
            other => panic!("expected SnapshotLoaded for root resync, got {other:?}"),
        }
        Ok(())
    }
}
