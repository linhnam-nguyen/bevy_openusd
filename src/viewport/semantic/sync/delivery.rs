use std::time::Instant;

use bevy::prelude::World;
use usd_bevy::{LiveRevision, LiveStage, ProgressiveProjectionState, ProjectionReadiness};
use usd_model::SemanticSnapshot;

use crate::project::blob_store::PreparedMeshBlob;
use crate::project::ghost_cache::{prepare_render_blobs, prepare_render_blobs_for_entities};
use crate::project::recovery::RecoverySettings;
use crate::viewport::api::RenderServerInterface;

use super::super::state::SemanticSyncState;
use super::action::SemanticSyncAction;
use super::delivery_worker::{
    PendingRuntimeDelivery, RuntimeDeliveryIdentity, RuntimeDeliveryRuntime,
};

pub(in crate::viewport::semantic) fn queue_runtime_delivery(
    world: &mut World,
    session_id: u64,
    live_revision: LiveRevision,
    snapshot: &SemanticSnapshot,
    prepared_blobs: Vec<PreparedMeshBlob>,
) {
    if world.get_resource::<RenderServerInterface>().is_none() {
        return;
    }
    if world.get_resource::<RuntimeDeliveryRuntime>().is_none() {
        world.init_resource::<RuntimeDeliveryRuntime>();
    }
    let projection_generation = world
        .get_resource::<ProgressiveProjectionState>()
        .map_or(0, ProgressiveProjectionState::generation);
    if let Some(mut runtime) = world.get_resource_mut::<RuntimeDeliveryRuntime>() {
        runtime.replace_pending(PendingRuntimeDelivery {
            identity: RuntimeDeliveryIdentity {
                session_id,
                live_revision,
                projection_generation,
            },
            snapshot: snapshot.clone(),
            prepared_blobs,
        });
    }
    if let Some(interface) = world.get_resource::<RenderServerInterface>() {
        interface.shared().clear_runtime_delivery();
    }
    flush_pending_runtime_delivery(world);
}

/// Submit the latest complete snapshot only when the projection is ready.
pub(crate) fn flush_pending_runtime_delivery(world: &mut World) {
    let Some(settings) = world.get_resource::<RecoverySettings>().cloned() else {
        return;
    };
    let projection = world
        .get_resource::<ProgressiveProjectionState>()
        .map(|state| (state.readiness(), state.generation()));
    let ready = projection.is_none_or(|(readiness, _)| readiness == ProjectionReadiness::Ready);
    if !ready {
        return;
    }

    let current_stage = world
        .get_non_send::<LiveStage>()
        .map(|live| (live.session_id(), live.current_revision()));
    let Some(mut runtime) = world.get_resource_mut::<RuntimeDeliveryRuntime>() else {
        return;
    };
    let Some(pending) = runtime.pending.as_mut() else {
        return;
    };
    if let Some((session_id, live_revision)) = current_stage
        && (pending.identity.session_id != session_id
            || pending.identity.live_revision != live_revision)
    {
        runtime.pending = None;
        return;
    }
    if let Some((_, generation)) = projection {
        pending.identity.projection_generation = generation;
    }
    let started = Instant::now();
    let submitted = runtime.submit_pending(&settings.project_root);
    if let Some(mut counters) =
        world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        counters.runtime_delivery_submit_ms += started.elapsed().as_secs_f64() * 1000.0;
    }
    if !submitted {
        bevy::log::debug!("[runtime-delivery] worker queue is closed");
    }
}

/// Publish only a worker result that still belongs to current authority.
pub(crate) fn drain_runtime_delivery_results(world: &mut World) {
    let Some(interface) = world
        .get_resource::<RenderServerInterface>()
        .map(RenderServerInterface::shared)
    else {
        return;
    };
    let current_stage = world
        .get_non_send::<LiveStage>()
        .map(|live| (live.session_id(), live.current_revision()));
    let projection = world
        .get_resource::<ProgressiveProjectionState>()
        .map(|state| (state.readiness(), state.generation()));
    let Some(runtime) = world.get_resource::<RuntimeDeliveryRuntime>() else {
        return;
    };
    let (results, queue_high_water) = (runtime.drain_results(), runtime.queue_stats().1);
    let result_backpressure = runtime.take_result_backpressure();
    if let Some(mut counters) =
        world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        counters.runtime_delivery_result_backpressure += result_backpressure;
    }
    for result in results {
        if let Some(mut counters) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            counters.runtime_delivery_worker_ms += result.worker_ms;
            counters.runtime_delivery_blob_reads += result.blob_reads;
            counters.runtime_delivery_bytes += result.bytes;
            counters.runtime_delivery_queue_high_water = counters
                .runtime_delivery_queue_high_water
                .max(queue_high_water);
        }
        let current_generation = projection.map_or(0, |(_, generation)| generation);
        let ready = projection.is_none_or(|(readiness, _)| readiness == ProjectionReadiness::Ready);
        let current_identity_matches = current_stage.is_none_or(|(session_id, revision)| {
            result.identity.session_id == session_id && result.identity.live_revision == revision
        }) && ready
            && result.identity.projection_generation == current_generation;
        if !current_identity_matches {
            let same_stage_revision = current_stage.is_some_and(|(session_id, revision)| {
                result.identity.session_id == session_id
                    && result.identity.live_revision == revision
            });
            if same_stage_revision
                && result.identity.projection_generation != current_generation
                && let Some(snapshot) = world
                    .get_resource::<SemanticSyncState>()
                    .and_then(|state| state.snapshot.clone())
                && let Some(mut runtime) = world.get_resource_mut::<RuntimeDeliveryRuntime>()
                && runtime.pending.is_none()
            {
                runtime.replace_pending(PendingRuntimeDelivery {
                    identity: RuntimeDeliveryIdentity {
                        session_id: result.identity.session_id,
                        live_revision: result.identity.live_revision,
                        projection_generation: current_generation,
                    },
                    snapshot: snapshot.as_ref().clone(),
                    // The stale worker already persisted its prepared bytes.
                    // The retry only rebuilds the complete manifest/hierarchy.
                    prepared_blobs: Vec::new(),
                });
                if let Some(mut counters) = world
                    .get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
                {
                    counters.runtime_delivery_generation_retries += 1;
                }
            }
            bevy::log::debug!(
                session_id = result.identity.session_id,
                live_revision = result.identity.live_revision.0,
                projection_generation = result.identity.projection_generation,
                "[runtime-delivery] stale result rejected"
            );
            continue;
        }
        match result.bundle {
            Ok(bundle) => {
                let (manifest, blobs) =
                    crate::project::runtime_delivery::into_delivery_parts(bundle);
                if let Err(error) = interface.publish_runtime_delivery(manifest, blobs) {
                    interface.clear_runtime_delivery();
                    bevy::log::warn!("[runtime-delivery] bundle publication rejected: {error:?}");
                }
            }
            Err(error) => {
                interface.clear_runtime_delivery();
                bevy::log::warn!("[runtime-delivery] bundle construction failed: {error}");
            }
        }
    }
}

pub(in crate::viewport::semantic) fn attach_render_blobs_to_action(
    world: &mut World,
    action: &mut SemanticSyncAction,
    live_revision: LiveRevision,
    root_count: usize,
) -> Vec<PreparedMeshBlob> {
    match action {
        SemanticSyncAction::Replace(snapshot) => prepare_render_blobs(world, snapshot),
        SemanticSyncAction::Delta(update) => {
            let Some(map) = world.get_resource::<usd_bevy::PrimEntities>() else {
                bevy::log::warn!(
                    target: "ghost_cache",
                    resync_fallback_reason = "missing_prim_entities_index",
                    root_count = root_count,
                    live_revision = live_revision.0,
                    "[ghost-cache] PrimEntities resource missing from world; falling back to full attach_render_blobs"
                );
                let prepared = prepare_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return prepared;
            };

            // Partial index corruption: PrimEntities exists, but an affected geometry prim has no index mapping
            let has_missing_mapping = update.request.upserts.iter().any(|entity| {
                entity
                    .geometry
                    .as_ref()
                    .is_some_and(|g| g.render_blob.is_none())
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
                let prepared = prepare_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return prepared;
            }

            // Enrich only affected upserted semantic entities
            let prepared = prepare_render_blobs_for_entities(world, &mut update.request.upserts);
            // Copy enriched upserts back into update.snapshot.entities
            for upsert in &update.request.upserts {
                if let Some(entity) = update.snapshot.entities.get_mut(&upsert.key) {
                    entity.geometry = upsert.geometry.clone();
                }
            }
            prepared
        }
    }
}
