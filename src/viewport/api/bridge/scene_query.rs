use std::time::Instant;

use bevy::prelude::*;
use viewport_protocol::{
    HierarchySource, PROTOCOL_VERSION, ViewportCommand, ViewportEvent, ViewportEventEnvelope,
};

use super::ViewerSettingsState;
use super::helpers::{build_read_model, reject};
use super::state::{SceneSearchRequest, SceneSearchRequests};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::camera::{CameraMount, CameraOrientationState};
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::{LoaderTuning, Spawned, StageHandle, StageInfo};

/// Drains hierarchy-search-worker responses and publishes search results.
pub(super) fn publish_scene_query_results(
    scene_query: Res<SceneQueryService>,
    mut search_requests: ResMut<SceneSearchRequests>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    for result in scene_query.drain_results() {
        let Some(request) = search_requests.pending.remove(&result.request_id) else {
            // The read model will reject a response whose request is no
            // longer current; dropping it here also bounds pending metadata.
            continue;
        };
        if let Some(ref mut counters) = counters {
            counters.query_results += 1;
            counters.record_query_latency_ms(request.submitted_at.elapsed().as_secs_f64() * 1000.0);
        }
        let event = match result.matches {
            super::super::scene_query::SearchMatches::Scene(matches) => {
                let matches = matches
                    .into_iter()
                    .filter_map(|result| result.into_scene_search_match())
                    .collect();
                ViewportEvent::SearchResults {
                    query: result.query,
                    offset: result.offset,
                    total: result.total,
                    matches,
                    has_more: result.has_more,
                }
            }
            super::super::scene_query::SearchMatches::Generic(matches) => {
                ViewportEvent::HierarchySearchResults {
                    source: result.source,
                    query: result.query,
                    offset: result.offset,
                    total: result.total,
                    matches,
                    has_more: result.has_more,
                }
            }
        };
        outbox.push(ViewportEventEnvelope::new(Some(result.request_id), event));
    }
}

/// Routes scene-query commands to the current hierarchy projection.
pub(super) fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    scene_query: Res<SceneQueryService>,
    mut search_requests: ResMut<SceneSearchRequests>,
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
            ViewportCommand::RequestHierarchyChildren {
                source,
                parent_id,
                page,
                page_size,
            } => {
                if source != HierarchySource::Prim {
                    reject(
                        &mut outbox,
                        request_id,
                        "BIM classification hierarchy provider is not active".to_owned(),
                    );
                    continue;
                }
                match scene_index.hierarchy_children_page(parent_id.as_ref(), page, page_size) {
                    Ok(page) => outbox.push(ViewportEventEnvelope::new(
                        Some(request_id),
                        ViewportEvent::HierarchyChildren { source, page },
                    )),
                    Err(error) => reject(&mut outbox, request_id, error),
                }
            }
            ViewportCommand::SearchScene {
                query,
                offset,
                limit,
            } => {
                let query_text = query.clone();
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    scene_index.hierarchy_snapshot(),
                    HierarchySource::Prim,
                    false,
                ) {
                    // Search is a single latest-query projection in the
                    // viewport read model. Dropping older metadata here also
                    // makes worker-side query coalescing safe: superseded
                    // responses are ignored when they arrive.
                    search_requests.pending.clear();
                    search_requests.pending.insert(
                        request_id,
                        SceneSearchRequest {
                            query: query_text,
                            offset,
                            submitted_at: Instant::now(),
                        },
                    );
                    if let Some(ref mut counters) = counters {
                        counters.query_requests += 1;
                    }
                } else {
                    if let Some(ref mut counters) = counters {
                        counters.query_failures += 1;
                    }
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy search worker is unavailable".to_owned(),
                    );
                }
            }
            ViewportCommand::SearchHierarchy {
                source,
                query,
                offset,
                limit,
            } => {
                if source != HierarchySource::Prim {
                    reject(
                        &mut outbox,
                        request_id,
                        "BIM classification hierarchy provider is not active".to_owned(),
                    );
                    continue;
                }
                let query_text = query.clone();
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    scene_index.hierarchy_snapshot(),
                    source,
                    true,
                ) {
                    search_requests.pending.clear();
                    search_requests.pending.insert(
                        request_id,
                        SceneSearchRequest {
                            query: query_text,
                            offset,
                            submitted_at: Instant::now(),
                        },
                    );
                    if let Some(ref mut counters) = counters {
                        counters.query_requests += 1;
                    }
                } else {
                    if let Some(ref mut counters) = counters {
                        counters.query_failures += 1;
                    }
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy search worker is unavailable".to_owned(),
                    );
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
    camera_orientation: Res<CameraOrientationState>,
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
            selection.revision(),
            &viewer_settings.0,
            &scene_index,
            &camera_mount,
            &camera_orientation.latest,
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
