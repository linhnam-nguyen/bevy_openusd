use std::time::Instant;

use bevy::prelude::*;
use viewport_protocol::{PROTOCOL_VERSION, ViewportCommand, ViewportEvent, ViewportEventEnvelope};

use super::ViewerSettingsState;
use super::helpers::{build_read_model, reject};
use super::state::{SemanticSearchRequest, SemanticSearchRequests};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::camera::CameraMount;
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::semantic::{SemanticQuery, SemanticWorkingStore};
use crate::viewport::session::{LoaderTuning, Spawned, StageHandle, StageInfo};

/// Drains semantic-worker responses and publishes search results.
pub(super) fn publish_semantic_query_results(
    semantic_store: Res<SemanticWorkingStore>,
    scene_index: Res<SceneAnchorIndex>,
    mut search_requests: ResMut<SemanticSearchRequests>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    use crate::viewport::semantic::SemanticResponse;
    for response in semantic_store.drain_responses() {
        match response {
            SemanticResponse::QueryResult { request_id, result } => {
                let Some(request) = search_requests.pending.remove(&request_id) else {
                    // The read model will reject a response whose request is
                    // no longer current; dropping it here also bounds the
                    // bridge-side pending request map.
                    continue;
                };
                if let Some(ref mut counters) = counters {
                    counters.query_results += 1;
                    counters.record_query_latency_ms(
                        request.submitted_at.elapsed().as_secs_f64() * 1000.0,
                    );
                }
                let matches = result
                    .rows
                    .iter()
                    .filter_map(|row| scene_index.search_match_for_path(&row.prim_path))
                    .collect();
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::SearchResults {
                        query: request.query,
                        offset: request.offset,
                        total: result.total,
                        matches,
                        has_more: result.has_more,
                    },
                ));
            }
            SemanticResponse::Failed {
                request_id,
                operation,
                error,
            } => {
                if let Some(request) = search_requests.pending.remove(&request_id) {
                    if let Some(ref mut counters) = counters {
                        counters.query_failures += 1;
                        counters.record_query_latency_ms(
                            request.submitted_at.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                    reject(
                        &mut outbox,
                        request_id,
                        format!("semantic {operation} failed: {error}"),
                    );
                } else {
                    warn!("[semantic-worker] {operation} failed: {error}");
                }
            }
            SemanticResponse::SnapshotLoaded { .. } | SemanticResponse::DeltaApplied { .. } => {}
        }
    }
}

/// Routes scene-query commands to the scene index or semantic worker.
pub(super) fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    semantic_store: Res<SemanticWorkingStore>,
    mut search_requests: ResMut<SemanticSearchRequests>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    for envelope in inbox.take_scene_query_commands() {
        let request_id = envelope.request_id;
        if envelope.protocol_version != PROTOCOL_VERSION {
            reject(
                &mut outbox,
                request_id,
                format!(
                    "unsupported protocol version {}; expected {}",
                    envelope.protocol_version, PROTOCOL_VERSION
                ),
            );
            continue;
        }

        match envelope.command {
            ViewportCommand::RequestSceneChildren {
                parent,
                page,
                page_size,
            } => outbox.push(ViewportEventEnvelope::new(
                Some(request_id),
                ViewportEvent::SceneChildren {
                    page: scene_index.children_page(parent.as_ref(), page, page_size),
                },
            )),
            ViewportCommand::SearchScene {
                query,
                offset,
                limit,
            } => {
                let query_text = query.clone();
                match semantic_store.try_submit_query(
                    request_id.clone(),
                    SemanticQuery {
                        text: Some(query),
                        offset,
                        limit,
                        ..Default::default()
                    },
                ) {
                    Ok(()) => {
                        // Search is a single latest-query projection in the
                        // viewport read model. Dropping older metadata here also
                        // makes worker-side query coalescing safe: superseded
                        // responses are ignored when they arrive.
                        search_requests.pending.clear();
                        search_requests.pending.insert(
                            request_id,
                            SemanticSearchRequest {
                                query: query_text,
                                offset,
                                submitted_at: Instant::now(),
                            },
                        );
                        if let Some(ref mut counters) = counters {
                            counters.query_requests += 1;
                        }
                    }
                    Err(error) => {
                        if let Some(ref mut counters) = counters {
                            counters.query_failures += 1;
                            if matches!(
                                error,
                                crate::viewport::semantic::SemanticSubmitError::QueueFull
                            ) {
                                counters.query_saturations += 1;
                            }
                        }
                        let message = match error {
                            crate::viewport::semantic::SemanticSubmitError::QueueFull => {
                                "semantic search worker queue is full"
                            }
                            crate::viewport::semantic::SemanticSubmitError::WorkerClosed => {
                                "semantic search worker is unavailable"
                            }
                        };
                        reject(&mut outbox, request_id, message.to_owned());
                    }
                }
            }
            _ => unreachable!("scene query inbox only contains query commands"),
        }
    }
}

/// Emits lifecycle changes independently of who initiated the load. That
/// makes manual reloads, file-watcher reloads, and future host commands all
/// observable through the same public event.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_stage_load_state(
    stage: Option<Res<StageHandle>>,
    spawned: Res<Spawned>,
    stage_info: Res<StageInfo>,
    selection: Res<SelectedTargets>,
    viewer_settings: Res<ViewerSettingsState>,
    scene_index: Res<SceneAnchorIndex>,
    camera_mount: Res<CameraMount>,
    clock: Res<UsdStageTime>,
    toggles: Res<DisplayToggles>,
    tuning: Res<LoaderTuning>,
    physics: Res<PhysicsActive>,
    mut last: Local<Option<(viewport_protocol::StageLoadState, u64)>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    use viewport_protocol::{StageLoadState, ViewportEvent};
    let state = match stage {
        None => StageLoadState::Idle,
        Some(stage) => match &stage.error {
            Some(error) => StageLoadState::Failed {
                message: error.clone(),
            },
            None if spawned.0 => StageLoadState::Ready,
            _ => StageLoadState::Loading,
        },
    };
    let state_changed = last.as_ref().is_none_or(|(previous, _)| previous != &state);
    let scene_changed = last
        .as_ref()
        .is_none_or(|(_, revision)| *revision != scene_index.revision());
    if state_changed || (matches!(state, StageLoadState::Ready) && scene_changed) {
        if state_changed {
            outbox.push(ViewportEventEnvelope::new(
                None,
                ViewportEvent::StageLoadStateChanged {
                    state: state.clone(),
                },
            ));
        }
        let snapshot = build_read_model(
            &stage_info,
            spawned.0 && matches!(state, StageLoadState::Ready),
            &selection.0,
            &viewer_settings.0,
            &scene_index,
            &camera_mount,
            &clock,
            &toggles,
            &tuning,
            physics.0,
        );
        info!(
            "[viewport-scene] publishing {:?} snapshot: total_prims={} total_roots={} payload_prims={}",
            state,
            snapshot.scene.total_prims,
            snapshot.scene.total_roots,
            snapshot.scene.prims.len()
        );
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snapshot),
            },
        ));
        *last = Some((state, scene_index.revision()));
    }
}
