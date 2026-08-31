use std::time::Instant;

use bevy::prelude::*;
use viewport_protocol::{
    HierarchySource, PROTOCOL_VERSION, ViewportCommand, ViewportEvent, ViewportEventEnvelope,
};

use super::ViewerSettingsState;
use super::helpers::{build_read_model, reject};
use super::state::{SceneSearchRequest, SceneSearchRequests};
use super::{bim_properties, bim_search};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::api::{
    ActiveHierarchyProvider, BimProvenanceService, CurrentHierarchyProjection, SceneAnchorIndex,
    ViewportCommandInbox, ViewportEventOutbox, refresh_projection_visibility,
};
use crate::viewport::camera::{CameraMount, CameraOrientationState};
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{ClassificationColorPlan, SelectedTargets};
use crate::viewport::session::{LoaderTuning, Spawned, StageHandle, StageInfo};

pub(super) use super::scene_query_results::publish_scene_query_results;
/// Routes scene-query commands to the current hierarchy projection.
pub(super) fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    mut provider: Option<ResMut<ActiveHierarchyProvider>>,
    semantic: Option<Res<crate::viewport::semantic::SemanticSyncState>>,
    semantic_diff: Option<Res<crate::viewport::semantic::SemanticDiffState>>,
    selection: Option<Res<SelectedTargets>>,
    stage_handle: Option<Res<StageHandle>>,
    scene_query: Res<SceneQueryService>,
    bim_provenance: Option<Res<BimProvenanceService>>,
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
                if source != current_projection.source() {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy provider {source:?} is not active"),
                    );
                    continue;
                }
                match current_projection.children_page(parent_id.as_ref(), page, page_size) {
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
                if current_projection.source() != HierarchySource::Prim {
                    reject(
                        &mut outbox,
                        request_id,
                        "prim scene search provider is not active".to_owned(),
                    );
                    continue;
                }
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    current_projection.snapshot(),
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
                if source != current_projection.source() {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy provider {source:?} is not active"),
                    );
                    continue;
                }
                let query_text = query.clone();
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    current_projection.snapshot(),
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
            ViewportCommand::SearchBim { query } => {
                bim_search::dispatch(
                    request_id,
                    query,
                    semantic.as_ref().map(|state| &**state),
                    &scene_query,
                    &mut search_requests,
                    &mut outbox,
                    &mut counters,
                );
            }
            ViewportCommand::RequestBimProperties => {
                bim_properties::dispatch(
                    request_id,
                    selection.as_deref(),
                    semantic.as_deref(),
                    semantic_diff.as_deref(),
                    &mut outbox,
                );
            }
            ViewportCommand::RequestBimPropertyProvenance {
                target,
                property,
                history_head,
            } => {
                let Some(stage_handle) = stage_handle.as_deref() else {
                    bim_properties::emit_unavailable(
                        request_id,
                        target,
                        property,
                        history_head,
                        &mut outbox,
                    );
                    continue;
                };
                let Some(bim_provenance) = bim_provenance.as_deref() else {
                    bim_properties::emit_unavailable(
                        request_id,
                        target,
                        property,
                        history_head,
                        &mut outbox,
                    );
                    continue;
                };
                bim_properties::submit_provenance(
                    request_id,
                    target,
                    property,
                    history_head,
                    semantic.as_deref(),
                    semantic_diff.as_deref(),
                    &stage_handle.path,
                    &bim_provenance,
                    &mut outbox,
                );
            }
            ViewportCommand::SetHierarchySource {
                source,
                classification_recipe,
            } => {
                let Some(provider) = provider.as_deref_mut() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy provider state is unavailable".to_owned(),
                    );
                    continue;
                };
                let projection = match source {
                    HierarchySource::Prim => Ok(scene_index.prim_projection()),
                    HierarchySource::BimClassification => match (
                        classification_recipe.as_ref(),
                        semantic.as_ref().and_then(|state| state.snapshot()),
                    ) {
                        (Some(recipe), Some(snapshot)) => {
                            crate::viewport::bim::BimReadService::new(snapshot)
                                .classification_projection(recipe)
                                .map_err(|error| error.to_string())
                        }
                        _ => Err(
                            "BIM classification recipe or semantic snapshot is unavailable"
                                .to_owned(),
                        ),
                    },
                };
                match projection {
                    Ok(projection) => {
                        let mut projection = projection;
                        refresh_projection_visibility(&mut projection, &scene_index);
                        *current_projection = projection;
                        provider.set(source, classification_recipe);
                    }
                    Err(error) => reject(&mut outbox, request_id, error),
                }
            }
            _ => unreachable!("scene query inbox only contains query commands"),
        }
    }
}

/// Rebuilds the active virtual provider only when the semantic snapshot
/// changes. Prim projection refresh remains owned by `SceneAnchorIndex`.
pub(super) fn refresh_active_hierarchy_projection(
    provider: Res<ActiveHierarchyProvider>,
    semantic: Res<crate::viewport::semantic::SemanticSyncState>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    mut color_plan: Option<ResMut<ClassificationColorPlan>>,
) {
    if provider.source() != HierarchySource::BimClassification || !semantic.is_changed() {
        return;
    }
    let (Some(recipe), Some(snapshot)) = (provider.classification_recipe(), semantic.snapshot())
    else {
        return;
    };
    let mut service = crate::viewport::bim::BimReadService::new(snapshot);
    let color_intent = color_plan.as_ref().and_then(|plan| plan.intent());
    let color_entries = color_intent.as_ref().map(|intent| {
        match service.classification_color_entries(recipe, intent) {
            Ok(entries) => entries,
            Err(error) => {
                bevy::log::warn!(
                    error = %error,
                    "classification color intent could not be materialized"
                );
                Vec::new()
            }
        }
    });
    let Ok(mut projection) = service.classification_projection(recipe) else {
        return;
    };
    refresh_projection_visibility(&mut projection, &scene_index);
    *current_projection = projection;
    if let (Some(plan), Some(entries)) = (color_plan.as_deref_mut(), color_entries) {
        plan.replace_entries(entries);
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
