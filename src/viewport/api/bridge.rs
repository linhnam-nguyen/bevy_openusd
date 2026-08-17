use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{
    CameraSource, CurveTuning as ProtocolCurveTuning, EditorOperation, EditorPrimReadModel,
    EditorStateReadModel, EditorValue, GroundGridOrigin, OverlayKind, PROTOCOL_VERSION,
    PresentationReadModel, SceneAnchor, SelectionReadModel, StageLoadState, StageReadModel,
    TimelineReadModel, ViewportCommand, ViewportEvent, ViewportEventEnvelope, ViewportReadModel,
};

use super::{
    SceneAnchorIndex, SceneQueryService, ViewportCommandInbox, ViewportEventOutbox,
    ViewportReadModelState, ViewportTreeCommand, ViewportTreeCommandInbox,
};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::camera::{ArcballCamera, CameraMount, FlyTo};
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::SelectedPrim;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::semantic::SemanticWorkingStore;
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageHandle, StageInfo};

/// Installs the in-process implementation of the public viewport contract.
pub(crate) struct ViewportBridgePlugin;

#[derive(Resource, Default)]
struct EditorHistories {
    authoring: usd_bevy::authoring::EditHistory,
    transforms: usd_bevy::TransformHistory,
    undo_domains: Vec<EditorHistoryDomain>,
    redo_domains: Vec<EditorHistoryDomain>,
}

#[derive(Clone, Copy)]
enum EditorHistoryDomain {
    Authoring,
    Transform,
}

impl EditorHistories {
    fn record(&mut self, domain: EditorHistoryDomain) {
        self.undo_domains.push(domain);
        self.redo_domains.clear();
    }

    fn state(&self) -> EditorStateReadModel {
        EditorStateReadModel {
            can_undo: !self.undo_domains.is_empty(),
            can_redo: !self.redo_domains.is_empty(),
        }
    }
}

/// Explicit ordering points around the public viewport bridge.
///
/// Native transports enqueue work before [`Self::ApplyCommands`] and drain
/// resulting events after [`Self::PublishStageLoadState`]. This preserves the
/// existing Frost behavior while avoiding a blocking pipe operation inside an
/// ECS command system.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ViewportBridgeSet {
    RefreshSceneIndex,
    ApplyCommands,
    ApplyTreeCommands,
    PublishStageLoadState,
    ReduceEvents,
}

impl Plugin for ViewportBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportReadModelState>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SceneQueryService>()
            .init_resource::<SemanticWorkingStore>()
            .init_resource::<EditorHistories>()
            .add_systems(Startup, emit_viewport_ready)
            .configure_sets(
                Update,
                (
                    ViewportBridgeSet::RefreshSceneIndex,
                    ViewportBridgeSet::ApplyCommands,
                    ViewportBridgeSet::ApplyTreeCommands,
                    ViewportBridgeSet::PublishStageLoadState,
                    ViewportBridgeSet::ReduceEvents,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                super::scene_index::refresh_scene_anchor_index
                    .in_set(ViewportBridgeSet::RefreshSceneIndex),
            )
            .add_systems(
                Update,
                (
                    publish_scene_query_results,
                    dispatch_scene_query_commands,
                    apply_viewport_commands,
                )
                    .chain()
                    .in_set(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                apply_tree_commands.in_set(ViewportBridgeSet::ApplyTreeCommands),
            )
            .add_systems(
                Update,
                publish_stage_load_state.in_set(ViewportBridgeSet::PublishStageLoadState),
            )
            .add_systems(
                Update,
                reduce_authoritative_events.in_set(ViewportBridgeSet::ReduceEvents),
            );
    }
}

/// Reduces each newly emitted authoritative event for the local Frost
/// reference adapter. The transport-delivery queue remains untouched.
fn reduce_authoritative_events(
    mut outbox: ResMut<ViewportEventOutbox>,
    mut read_model: ResMut<ViewportReadModelState>,
) {
    for event in outbox.take_published() {
        read_model.apply(&event);
    }
}

fn publish_scene_query_results(
    query_service: Res<SceneQueryService>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    for result in query_service.drain_results() {
        outbox.push(ViewportEventEnvelope::new(
            Some(result.request_id),
            ViewportEvent::SearchResults {
                query: result.query,
                offset: result.offset,
                total: result.total,
                matches: result.matches,
                has_more: result.has_more,
            },
        ));
    }
}

fn emit_viewport_ready(mut outbox: ResMut<ViewportEventOutbox>) {
    outbox.push(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
}

fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    query_service: Res<SceneQueryService>,
    mut outbox: ResMut<ViewportEventOutbox>,
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
                if !query_service.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    scene_index.nodes_snapshot(),
                ) {
                    reject(
                        &mut outbox,
                        request_id,
                        "scene search worker is unavailable".to_owned(),
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
fn publish_stage_load_state(
    stage: Option<Res<StageHandle>>,
    spawned: Res<Spawned>,
    stage_info: Res<StageInfo>,
    selected: Res<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    camera_mount: Res<CameraMount>,
    clock: Res<UsdStageTime>,
    toggles: Res<DisplayToggles>,
    tuning: Res<LoaderTuning>,
    physics: Res<PhysicsActive>,
    mut last: Local<Option<(StageLoadState, u64)>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
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
            &selected,
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
            ViewportEvent::Snapshot { state: snapshot },
        ));
        *last = Some((state, scene_index.revision()));
    }
}

/// Applies commands whose state does not require a tree traversal. Tree
/// commands are forwarded to the next system after the scene index refreshes.
#[allow(clippy::too_many_arguments)]
fn apply_viewport_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut reload: ResMut<ReloadRequest>,
    mut selected: ResMut<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    mut tree_commands: ResMut<ViewportTreeCommandInbox>,
    mut camera_mount: ResMut<CameraMount>,
    mut clock: ResMut<UsdStageTime>,
    mut toggles: ResMut<DisplayToggles>,
    mut tuning: ResMut<LoaderTuning>,
    mut physics: ResMut<PhysicsActive>,
    mut histories: ResMut<EditorHistories>,
    stage: Option<NonSend<LiveStage>>,
    stage_info: Res<StageInfo>,
    spawned: Res<Spawned>,
) {
    while let Some(envelope) = inbox.pop() {
        let request_id = envelope.request_id.clone();
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
            ViewportCommand::RequestSnapshot => emit_snapshot(
                &mut outbox,
                request_id,
                &stage_info,
                &spawned,
                &selected,
                &scene_index,
                &camera_mount,
                &clock,
                &toggles,
                &tuning,
                physics.0,
            ),
            ViewportCommand::RequestSceneChildren { .. } | ViewportCommand::SearchScene { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "scene query command was not dispatched".to_owned(),
                );
            }
            ViewportCommand::ReloadSession => {
                reload.requested = true;
                *histories = EditorHistories::default();
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
            }
            ViewportCommand::SelectTarget { target } => {
                let selection = match target {
                    None => {
                        selected.0 = None;
                        SelectionReadModel { target: None }
                    }
                    Some(anchor) => match resolve_anchor(&anchor, &scene_index) {
                        Ok(entity) => {
                            selected.0 = Some(entity);
                            SelectionReadModel {
                                target: Some(anchor),
                            }
                        }
                        Err(reason) => {
                            reject(&mut outbox, request_id, reason);
                            continue;
                        }
                    },
                };
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::SelectionChanged { selection },
                ));
            }
            ViewportCommand::FocusTarget { target, mode } => {
                tree_commands.push(ViewportTreeCommand::Focus {
                    request_id,
                    target,
                    mode,
                });
            }
            ViewportCommand::SetSubtreeVisibility { target, visible } => {
                tree_commands.push(ViewportTreeCommand::SetSubtreeVisibility {
                    request_id,
                    target,
                    visible,
                });
            }
            ViewportCommand::SetVariantSelection {
                prim_path,
                set_name,
                option,
            } => {
                tuning
                    .variants
                    .insert((prim_path.clone(), set_name.clone()), option.clone());
                if let Some(stage) = stage.as_deref() {
                    stage.mark_authored(prim_path.clone());
                    if let Err(error) = histories.authoring.set_variant(
                        &stage.stage,
                        &prim_path,
                        &set_name,
                        &option,
                    ) {
                        reject(&mut outbox, request_id, error.to_string());
                        continue;
                    }
                    histories.record(EditorHistoryDomain::Authoring);
                }
                emit_snapshot(
                    &mut outbox,
                    request_id.clone(),
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::SetVariantSelection,
                    vec![format!("{prim_path}.{set_name}")],
                    &histories,
                );
            }
            ViewportCommand::ResetVariantSelection {
                prim_path,
                set_name,
            } => {
                tuning
                    .variants
                    .remove(&(prim_path.clone(), set_name.clone()));
                // The current OpenUSD binding exposes authoring of an explicit
                // selection. Reset is represented as a local presentation
                // reset until clear-selection authoring is added upstream.
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
            }
            ViewportCommand::SetCameraSource { source } => {
                *camera_mount = match &source {
                    CameraSource::Arcball => CameraMount::Arcball,
                    CameraSource::Authored { prim_path } => CameraMount::Mounted {
                        prim_path: prim_path.clone(),
                    },
                };
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::CameraSourceChanged { source },
                ));
            }
            ViewportCommand::SetPlayback { playing } => {
                clock.playing = playing;
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::TimelineChanged {
                        timeline: timeline_read_model(&clock),
                    },
                ));
            }
            ViewportCommand::Seek { seconds } => {
                clock.seconds = seconds.clamp(0.0, clock.duration_seconds());
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::TimelineChanged {
                        timeline: timeline_read_model(&clock),
                    },
                ));
            }
            ViewportCommand::SetOverlay { overlay, enabled } => {
                set_overlay(&mut toggles, overlay, enabled);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetGroundGridOrigin { origin } => {
                toggles.ground_grid_origin = origin;
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetPrimMarkerBias { bias } => {
                toggles.prim_marker_bias = bias.clamp(0.0, 5.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetLightIntensity { scale } => {
                toggles.light_intensity_scale = scale.clamp(0.0, 5.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetCurveTuning { tuning: next } => {
                tuning.curves.default_radius = next.default_radius.clamp(0.001, 0.2);
                tuning.curves.ring_segments = next.ring_segments.clamp(3, 24);
                tuning.curves.point_scale = next.point_scale.clamp(0.05, 4.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetPhysicsRunning { running } => {
                physics.0 = running;
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::PhysicsChanged { running },
                ));
            }
            ViewportCommand::DefinePrim { path, type_name } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(path.clone());
                if let Err(error) = histories.authoring.define(&stage.stage, &path, &type_name) {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                histories.record(EditorHistoryDomain::Authoring);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::DefinePrim,
                    vec![path],
                    &histories,
                );
            }
            ViewportCommand::RemovePrim { path } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(path.clone());
                let removed = match usd_bevy::authoring::remove_prim(&stage.stage, &path) {
                    Ok(removed) => removed,
                    Err(error) => {
                        reject(&mut outbox, request_id, error.to_string());
                        continue;
                    }
                };
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::RemovePrim,
                    removed.then_some(path).into_iter().collect(),
                    &histories,
                );
            }
            ViewportCommand::RenamePrim { path, new_name } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(path.clone());
                if let Err(error) = histories.authoring.rename(&stage.stage, &path, &new_name) {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                histories.record(EditorHistoryDomain::Authoring);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::RenamePrim,
                    vec![path],
                    &histories,
                );
            }
            ViewportCommand::ReparentPrim { path, new_parent } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(path.clone());
                if let Err(error) = histories
                    .authoring
                    .reparent(&stage.stage, &path, &new_parent)
                {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                histories.record(EditorHistoryDomain::Authoring);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::ReparentPrim,
                    vec![path, new_parent],
                    &histories,
                );
            }
            ViewportCommand::MovePrim { old_path, new_path } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(old_path.clone());
                if let Err(error) =
                    usd_bevy::authoring::move_prim(&stage.stage, &old_path, &new_path)
                {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::MovePrim,
                    vec![old_path, new_path],
                    &histories,
                );
            }
            ViewportCommand::SetAttribute {
                prim_path,
                name,
                type_name,
                value,
            } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                let value = match editor_value_to_usd(&type_name, &value) {
                    Ok(value) => value,
                    Err(error) => {
                        reject(&mut outbox, request_id, error);
                        continue;
                    }
                };
                stage.mark_authored(prim_path.clone());
                if let Err(error) =
                    histories
                        .authoring
                        .set_attr(&stage.stage, &prim_path, &name, &type_name, value)
                {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                histories.record(EditorHistoryDomain::Authoring);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::SetAttribute,
                    vec![format!("{prim_path}.{name}")],
                    &histories,
                );
            }
            ViewportCommand::ClearAttribute { prim_path, name } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(prim_path.clone());
                if let Err(error) =
                    usd_bevy::authoring::clear_attribute(&stage.stage, &prim_path, &name)
                {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::ClearAttribute,
                    vec![format!("{prim_path}.{name}")],
                    &histories,
                );
            }
            ViewportCommand::SetTransform {
                prim_path,
                translation,
                rotation,
                scale,
            } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                stage.mark_authored(prim_path.clone());
                let transform = Transform {
                    translation: Vec3::from_array(translation),
                    rotation: Quat::from_array(rotation),
                    scale: Vec3::from_array(scale),
                };
                if let Err(error) = histories
                    .transforms
                    .author(&stage.stage, &prim_path, transform)
                {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                histories.record(EditorHistoryDomain::Transform);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::SetTransform,
                    vec![prim_path],
                    &histories,
                );
            }
            ViewportCommand::LoadPayload { prim_path } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                if !usd_bevy::authoring::prim_exists(&stage.stage, &prim_path) {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("prim {prim_path} does not exist"),
                    );
                    continue;
                }
                stage.load_payload(&prim_path);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::LoadPayload,
                    vec![prim_path],
                    &histories,
                );
            }
            ViewportCommand::UnloadPayload { prim_path } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                if !usd_bevy::authoring::prim_exists(&stage.stage, &prim_path) {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("prim {prim_path} does not exist"),
                    );
                    continue;
                }
                stage.unload_payload(&prim_path);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::UnloadPayload,
                    vec![prim_path],
                    &histories,
                );
            }
            ViewportCommand::UndoEditor => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                let Some(domain) = histories.undo_domains.pop() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "editor history is empty".to_owned(),
                    );
                    continue;
                };
                let result = match domain {
                    EditorHistoryDomain::Authoring => histories.authoring.undo(&stage.stage),
                    EditorHistoryDomain::Transform => histories.transforms.undo(&stage.stage),
                };
                match result {
                    Ok(true) => {
                        histories.redo_domains.push(domain);
                        emit_editor_completed(
                            &mut outbox,
                            request_id,
                            EditorOperation::Undo,
                            Vec::new(),
                            &histories,
                        );
                    }
                    Ok(false) => reject(
                        &mut outbox,
                        request_id,
                        "editor history is empty".to_owned(),
                    ),
                    Err(error) => {
                        histories.undo_domains.push(domain);
                        reject(&mut outbox, request_id, error.to_string());
                    }
                }
            }
            ViewportCommand::RedoEditor => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                let Some(domain) = histories.redo_domains.pop() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "editor redo history is empty".to_owned(),
                    );
                    continue;
                };
                let result = match domain {
                    EditorHistoryDomain::Authoring => histories.authoring.redo(&stage.stage),
                    EditorHistoryDomain::Transform => histories.transforms.redo(&stage.stage),
                };
                match result {
                    Ok(true) => {
                        histories.undo_domains.push(domain);
                        emit_editor_completed(
                            &mut outbox,
                            request_id,
                            EditorOperation::Redo,
                            Vec::new(),
                            &histories,
                        );
                    }
                    Ok(false) => reject(
                        &mut outbox,
                        request_id,
                        "editor redo history is empty".to_owned(),
                    ),
                    Err(error) => {
                        histories.redo_domains.push(domain);
                        reject(&mut outbox, request_id, error.to_string());
                    }
                }
            }
            ViewportCommand::SaveStageAs { filename } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                if let Err(error) = usd_bevy::authoring::save_stage_as(&stage.stage, &filename) {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::SaveStageAs,
                    Vec::new(),
                    &histories,
                );
            }
            ViewportCommand::ExportStage => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                let content = match usd_bevy::authoring::export_stage_string(&stage.stage) {
                    Ok(content) => content,
                    Err(error) => {
                        reject(&mut outbox, request_id, error.to_string());
                        continue;
                    }
                };
                emit_editor_export(&mut outbox, &request_id, &content);
                emit_editor_completed(
                    &mut outbox,
                    request_id,
                    EditorOperation::ExportStage,
                    Vec::new(),
                    &histories,
                );
            }
            ViewportCommand::QueryPrim { prim_path } => {
                let Some(stage) = stage.as_deref() else {
                    reject(&mut outbox, request_id, "stage is not loaded".to_owned());
                    continue;
                };
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::EditorPrimState {
                        prim: EditorPrimReadModel {
                            prim_path: prim_path.clone(),
                            exists: usd_bevy::authoring::prim_exists(&stage.stage, &prim_path),
                        },
                    },
                ));
            }
        }
    }
}

/// Applies focus and visibility actions after scene anchors have been mapped
/// to their private Bevy entities. Both selection and fly-to use the same
/// subtree bounds, so repeating the action does not progressively zoom toward
/// a prim's transform origin.
#[allow(clippy::too_many_arguments)]
fn apply_tree_commands(
    mut inbox: ResMut<ViewportTreeCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut selected: ResMut<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    cameras: Query<&ArcballCamera>,
    transforms: Query<&Transform>,
    child_of: Query<Option<&ChildOf>>,
    extents: Query<&usd_bevy::UsdLocalExtent>,
    aabbs: Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: Query<Option<&Mesh3d>>,
    children: Query<&Children>,
    mut visibility: Query<(Entity, &mut Visibility)>,
    mut fly_to: ResMut<FlyTo>,
) {
    while let Some(command) = inbox.pop() {
        match command {
            ViewportTreeCommand::Focus {
                request_id,
                target,
                mode,
            } => {
                let Some(entity) = scene_index.resolve(&target) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "target {} is not present in the active scene",
                            target.prim_path
                        ),
                    );
                    continue;
                };
                let Ok(camera) = cameras.single() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "cannot focus target before the active camera is ready".to_string(),
                    );
                    continue;
                };

                let Some((target_focus, target_distance)) = fit_params_for_entity(
                    entity,
                    &transforms,
                    &child_of,
                    &extents,
                    &aabbs,
                    &meshes,
                    &children,
                    camera.distance,
                ) else {
                    // A Mesh3d can exist for a frame before Bevy has produced
                    // its Aabb. Preserve the command and retry next frame so
                    // the camera never commits to the prim origin as a fake
                    // fit target.
                    inbox.push_front(ViewportTreeCommand::Focus {
                        request_id,
                        target,
                        mode,
                    });
                    break;
                };

                selected.0 = Some(entity);
                fly_to.start_focus = camera.focus;
                fly_to.start_distance = camera.distance;
                fly_to.target_focus = target_focus;
                fly_to.target_distance = target_distance;
                fly_to.start_yaw = None;
                fly_to.target_yaw = None;
                fly_to.start_elevation = None;
                fly_to.target_elevation = None;
                fly_to.duration = 0.4;
                fly_to.remaining = 0.4;

                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id.clone()),
                    ViewportEvent::SelectionChanged {
                        selection: SelectionReadModel {
                            target: Some(target.clone()),
                        },
                    },
                ));
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::CameraTransitionStarted { target, mode },
                ));
            }
            ViewportTreeCommand::SetSubtreeVisibility {
                request_id,
                target,
                visible,
            } => {
                let Some(root) = scene_index.resolve(&target) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "target {} is not present in the active scene",
                            target.prim_path
                        ),
                    );
                    continue;
                };

                set_subtree_visibility(root, &children, &mut visibility, visible);
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::PrimVisibilityChanged { target, visible },
                ));
            }
        }
    }
}

/// Matches the Frost tree's one-way descendant visibility change. Ancestors
/// and siblings remain untouched, and enabled descendants use `Visible`
/// rather than `Inherited` so a prior hidden parent cannot keep them hidden.
fn set_subtree_visibility(
    root: Entity,
    children: &Query<&Children>,
    visibility: &mut Query<(Entity, &mut Visibility)>,
    visible: bool,
) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok((_, mut current)) = visibility.get_mut(entity) {
            *current = if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter());
        }
    }
}

/// Computes the subtree bounds used by both public focus modes so a product
/// client frames the same target the same way.
fn fit_params_for_entity(
    root: Entity,
    transforms: &Query<&Transform>,
    child_of: &Query<Option<&ChildOf>>,
    extents: &Query<&usd_bevy::UsdLocalExtent>,
    aabbs: &Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: &Query<Option<&Mesh3d>>,
    children: &Query<&Children>,
    current_camera_distance: f32,
) -> Option<(Vec3, f32)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    let mut mesh_bounds_pending = false;
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        if transforms.get(entity).is_ok() {
            let matrix = world_matrix(entity, transforms, child_of)?;
            if let Ok(extent) = extents.get(entity) {
                include_bounds(
                    &mut min,
                    &mut max,
                    matrix,
                    Vec3::from_array(extent.min),
                    Vec3::from_array(extent.max),
                );
                found = true;
            } else if let Ok(Some(aabb)) = aabbs.get(entity) {
                include_bounds(
                    &mut min,
                    &mut max,
                    matrix,
                    Vec3::from(aabb.center - aabb.half_extents),
                    Vec3::from(aabb.center + aabb.half_extents),
                );
                found = true;
            } else if meshes.get(entity).ok().flatten().is_some() {
                mesh_bounds_pending = true;
            }
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter());
        }
    }

    if found {
        let center = (min + max) * 0.5;
        let size = (max - min).abs();
        let maximum_dimension = size.x.max(size.y).max(size.z).max(0.05);
        Some((center, (maximum_dimension * 1.6).clamp(0.2, 10_000.0)))
    } else if mesh_bounds_pending {
        None
    } else if transforms.get(root).is_ok() {
        Some((
            world_matrix(root, transforms, child_of)?.transform_point3(Vec3::ZERO),
            current_camera_distance.clamp(0.2, 10_000.0),
        ))
    } else {
        None
    }
}

/// Computes the current world matrix from local transforms instead of relying
/// on `GlobalTransform`, which is propagated later in the frame. This keeps a
/// selection command correct even when it arrives in the same frame as an
/// authored transform update.
fn world_matrix(
    entity: Entity,
    transforms: &Query<&Transform>,
    child_of: &Query<Option<&ChildOf>>,
) -> Option<Mat4> {
    let mut chain = Vec::new();
    let mut current = Some(entity);
    let mut guard = 0usize;
    while let Some(entity) = current {
        let transform = transforms.get(entity).ok()?;
        chain.push(transform.to_matrix());
        current = child_of.get(entity).ok().flatten().map(ChildOf::parent);
        guard += 1;
        if guard > 10_000 {
            return None;
        }
    }

    Some(
        chain
            .into_iter()
            .rev()
            .fold(Mat4::IDENTITY, |parent, local| parent * local),
    )
}

fn include_bounds(min: &mut Vec3, max: &mut Vec3, matrix: Mat4, local_min: Vec3, local_max: Vec3) {
    for index in 0..8 {
        let corner = Vec3::new(
            if index & 1 == 0 {
                local_min.x
            } else {
                local_max.x
            },
            if index & 2 == 0 {
                local_min.y
            } else {
                local_max.y
            },
            if index & 4 == 0 {
                local_min.z
            } else {
                local_max.z
            },
        );
        let world_corner = matrix.transform_point3(corner);
        *min = min.min(world_corner);
        *max = max.max(world_corner);
    }
}

fn resolve_anchor(anchor: &SceneAnchor, scene_index: &SceneAnchorIndex) -> Result<Entity, String> {
    scene_index.resolve(anchor).ok_or_else(|| {
        format!(
            "target {} is not present in the active scene",
            anchor.prim_path
        )
    })
}

fn set_overlay(toggles: &mut DisplayToggles, overlay: OverlayKind, enabled: bool) {
    match overlay {
        OverlayKind::GroundGrid => toggles.show_world_grid = enabled,
        OverlayKind::WorldAxes => toggles.show_world_axes = enabled,
        OverlayKind::PrimMarkers => toggles.show_prim_markers = enabled,
        OverlayKind::Skeleton => toggles.show_skeleton = enabled,
        OverlayKind::Physics => toggles.show_physics = enabled,
        OverlayKind::Colliders => toggles.show_colliders = enabled,
        OverlayKind::Wireframe => toggles.wireframe = enabled,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_snapshot(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    stage_info: &StageInfo,
    spawned: &Spawned,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::Snapshot {
            state: build_read_model(
                stage_info,
                spawned.0,
                selected,
                scene_index,
                camera_mount,
                clock,
                toggles,
                tuning,
                physics_running,
            ),
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn build_read_model(
    stage_info: &StageInfo,
    stage_loaded: bool,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) -> ViewportReadModel {
    ViewportReadModel {
        protocol_version: PROTOCOL_VERSION,
        stage: StageReadModel {
            display_name: stage_info.path.clone(),
            loaded: stage_loaded,
        },
        scene: scene_index.roots_read_model(),
        selection: SelectionReadModel {
            target: selected.0.and_then(|entity| scene_index.anchor_for(entity)),
        },
        camera_source: match camera_mount {
            CameraMount::Arcball => CameraSource::Arcball,
            CameraMount::Mounted { prim_path } => CameraSource::Authored {
                prim_path: prim_path.clone(),
            },
        },
        timeline: timeline_read_model(clock),
        presentation: presentation_read_model(toggles, tuning),
        physics_running,
    }
}

fn timeline_read_model(clock: &UsdStageTime) -> TimelineReadModel {
    TimelineReadModel {
        seconds: clock.seconds,
        playing: clock.playing,
        start_time_code: clock.start_time_code,
        end_time_code: clock.end_time_code,
        time_codes_per_second: clock.time_codes_per_second,
    }
}

fn presentation_read_model(
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) -> PresentationReadModel {
    PresentationReadModel {
        ground_grid: toggles.show_world_grid,
        ground_grid_origin: toggles.ground_grid_origin,
        world_axes: toggles.show_world_axes,
        prim_markers: toggles.show_prim_markers,
        prim_marker_bias: toggles.prim_marker_bias,
        skeleton: toggles.show_skeleton,
        physics: toggles.show_physics,
        colliders: toggles.show_colliders,
        wireframe: toggles.wireframe,
        light_intensity_scale: toggles.light_intensity_scale,
        curve_tuning: ProtocolCurveTuning {
            default_radius: tuning.curves.default_radius,
            ring_segments: tuning.curves.ring_segments,
            point_scale: tuning.curves.point_scale,
        },
    }
}

fn emit_presentation_changed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::PresentationChanged {
            presentation: presentation_read_model(toggles, tuning),
        },
    ));
}

fn reject(outbox: &mut ViewportEventOutbox, request_id: String, reason: String) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::CommandRejected { request_id, reason },
    ));
}

fn emit_editor_completed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    operation: EditorOperation,
    changed_paths: Vec<String>,
    histories: &EditorHistories,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::EditorCommandCompleted {
            operation,
            changed_paths,
            state: histories.state(),
        },
    ));
}

fn emit_editor_export(outbox: &mut ViewportEventOutbox, request_id: &str, content: &str) {
    // Leave room for the event and session envelopes under the transport's
    // 12 KiB application-message ceiling.
    const CHUNK_BYTES: usize = 8 * 1024;
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in content.chars() {
        if !current.is_empty() && current.len() + character.len_utf8() > CHUNK_BYTES {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }

    let export_id = format!("export-{request_id}");
    let chunk_count = chunks.len() as u32;
    for (chunk_index, content) in chunks.into_iter().enumerate() {
        outbox.push(ViewportEventEnvelope::new(
            Some(request_id.to_owned()),
            ViewportEvent::EditorStageExportChunk {
                export_id: export_id.clone(),
                chunk_index: chunk_index as u32,
                chunk_count,
                content,
            },
        ));
    }
}

fn editor_value_to_usd(
    type_name: &str,
    value: &EditorValue,
) -> Result<openusd::sdf::Value, String> {
    use openusd::sdf::Value;

    fn number(value: &EditorValue) -> Result<f64, String> {
        value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| "editor value must be a finite JSON number".to_owned())
    }
    fn integer(value: &EditorValue) -> Result<i64, String> {
        value
            .as_i64()
            .ok_or_else(|| "editor value must be a JSON integer".to_owned())
    }
    fn boolean(value: &EditorValue) -> Result<bool, String> {
        value
            .as_bool()
            .ok_or_else(|| "editor value must be a JSON boolean".to_owned())
    }
    fn text(value: &EditorValue) -> Result<String, String> {
        value
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| "editor value must be a JSON string".to_owned())
    }
    fn numbers<const N: usize>(value: &EditorValue) -> Result<[f64; N], String> {
        let values = value
            .as_array()
            .ok_or_else(|| format!("editor value must be an array of {N} numbers"))?;
        if values.len() != N {
            return Err(format!("editor value must contain exactly {N} numbers"));
        }
        values
            .iter()
            .map(number)
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| format!("editor value must contain exactly {N} numbers"))
    }
    fn values(value: &EditorValue) -> Result<&[EditorValue], String> {
        value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "editor array value must be a JSON array".to_owned())
    }

    let value = match type_name {
        "bool" => Value::Bool(boolean(value)?),
        "uchar" => Value::Uchar(
            integer(value)?
                .try_into()
                .map_err(|_| "uchar is outside the range 0..255".to_owned())?,
        ),
        "int" => Value::Int(
            integer(value)?
                .try_into()
                .map_err(|_| "int is outside the i32 range".to_owned())?,
        ),
        "uint" => Value::Uint(
            integer(value)?
                .try_into()
                .map_err(|_| "uint is outside the u32 range".to_owned())?,
        ),
        "int64" => Value::Int64(integer(value)?),
        "uint64" => Value::Uint64(
            integer(value)?
                .try_into()
                .map_err(|_| "uint64 cannot be negative".to_owned())?,
        ),
        "float" => Value::Float(number(value)? as f32),
        "double" => Value::Double(number(value)?),
        "string" => Value::String(text(value)?),
        "token" => Value::Token(text(value)?.as_str().into()),
        "asset" => Value::AssetPath(openusd::sdf::AssetPath::new(text(value)?)),
        "timecode" => Value::TimeCode(openusd::sdf::TimeCode(number(value)?)),
        "float2" => Value::Vec2f(numbers::<2>(value)?.map(|v| v as f32).into()),
        "float3" | "point3f" | "vector3f" | "normal3f" | "color3f" => {
            Value::Vec3f(numbers::<3>(value)?.map(|v| v as f32).into())
        }
        "float4" | "color4f" => Value::Vec4f(numbers::<4>(value)?.map(|v| v as f32).into()),
        "double2" => Value::Vec2d(numbers::<2>(value)?.into()),
        "double3" | "point3d" | "vector3d" | "normal3d" | "color3d" => {
            Value::Vec3d(numbers::<3>(value)?.into())
        }
        "double4" | "color4d" => Value::Vec4d(numbers::<4>(value)?.into()),
        "int2" => Value::Vec2i(numbers::<2>(value)?.map(|v| v as i32).into()),
        "int3" => Value::Vec3i(numbers::<3>(value)?.map(|v| v as i32).into()),
        "int4" => Value::Vec4i(numbers::<4>(value)?.map(|v| v as i32).into()),
        "quatf" => Value::Quatf(numbers::<4>(value)?.map(|v| v as f32).into()),
        "quatd" => Value::Quatd(numbers::<4>(value)?.into()),
        "matrix2d" => Value::Matrix2d(numbers::<4>(value)?.into()),
        "matrix3d" => Value::Matrix3d(numbers::<9>(value)?.into()),
        "matrix4d" => Value::Matrix4d(numbers::<16>(value)?.into()),
        "path" => Value::PathExpression(text(value)?),
        "bool[]" => Value::BoolVec(
            values(value)?
                .iter()
                .map(boolean)
                .collect::<Result<_, _>>()?,
        ),
        "int[]" => Value::IntVec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "int[] contains an out-of-range value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "uint[]" => Value::UintVec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "uint[] contains an out-of-range value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "int64[]" => Value::Int64Vec(
            values(value)?
                .iter()
                .map(integer)
                .collect::<Result<_, _>>()?,
        ),
        "uint64[]" => Value::Uint64Vec(
            values(value)?
                .iter()
                .map(|value| {
                    integer(value)?
                        .try_into()
                        .map_err(|_| "uint64[] contains a negative value".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        "float[]" => Value::FloatVec(
            values(value)?
                .iter()
                .map(|value| Ok(number(value)? as f32))
                .collect::<Result<_, String>>()?,
        ),
        "double[]" => Value::DoubleVec(
            values(value)?
                .iter()
                .map(number)
                .collect::<Result<_, _>>()?,
        ),
        "string[]" => Value::StringVec(values(value)?.iter().map(text).collect::<Result<_, _>>()?),
        "token[]" => Value::TokenVec(
            values(value)?
                .iter()
                .map(|value| Ok(text(value)?.as_str().into()))
                .collect::<Result<_, String>>()?,
        ),
        "asset[]" => Value::AssetPathVec(
            values(value)?
                .iter()
                .map(|value| Ok(openusd::sdf::AssetPath::new(text(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        "float3[]" => Value::Vec3fVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<3>(value)?.map(|v| v as f32).into()))
                .collect::<Result<_, String>>()?,
        ),
        "double3[]" => Value::Vec3dVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<3>(value)?.into()))
                .collect::<Result<_, String>>()?,
        ),
        "matrix4d[]" => Value::Matrix4dVec(
            values(value)?
                .iter()
                .map(|value| Ok(numbers::<16>(value)?.into()))
                .collect::<Result<_, String>>()?,
        ),
        _ => {
            return Err(format!(
                "unsupported USD editor attribute type: {type_name}"
            ));
        }
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SceneQueryService>()
            .init_resource::<ReloadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<CameraMount>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PhysicsActive>()
            .init_resource::<EditorHistories>()
            .init_resource::<Spawned>()
            .insert_resource(StageInfo {
                path: "fixtures/spinner.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands);
        app
    }

    #[test]
    fn commands_update_runtime_state_and_publish_correlated_events() {
        let mut app = command_test_app();
        let request_ids = {
            let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
            vec![
                inbox.send(ViewportCommand::SetOverlay {
                    overlay: OverlayKind::Wireframe,
                    enabled: true,
                }),
                inbox.send(ViewportCommand::SetPlayback { playing: true }),
                inbox.send(ViewportCommand::Seek { seconds: 999.0 }),
                inbox.send(ViewportCommand::SetPhysicsRunning { running: true }),
            ]
        };

        app.update();

        assert!(app.world().resource::<DisplayToggles>().wireframe);
        assert!(app.world().resource::<UsdStageTime>().playing);
        assert_eq!(app.world().resource::<UsdStageTime>().seconds, 1.0 / 24.0);
        assert!(app.world().resource::<PhysicsActive>().0);

        let events: Vec<_> =
            std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
                .collect();
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0].request_id.as_deref(),
            Some(request_ids[0].as_str())
        );
        assert!(matches!(
            events[0].event,
            ViewportEvent::PresentationChanged { .. }
        ));
        assert_eq!(
            events[1].request_id.as_deref(),
            Some(request_ids[1].as_str())
        );
        assert!(matches!(
            events[1].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[2].request_id.as_deref(),
            Some(request_ids[2].as_str())
        );
        assert!(matches!(
            events[2].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[3].request_id.as_deref(),
            Some(request_ids[3].as_str())
        );
        assert!(matches!(
            events[3].event,
            ViewportEvent::PhysicsChanged { running: true }
        ));
    }

    #[test]
    fn grid_origin_command_updates_presentation_state_and_event() {
        let mut app = command_test_app();
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetGroundGridOrigin {
                origin: GroundGridOrigin::WorldOrigin,
            },
        );

        app.update();

        assert_eq!(
            app.world().resource::<DisplayToggles>().ground_grid_origin,
            GroundGridOrigin::WorldOrigin
        );
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("grid-origin command publishes a presentation event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::PresentationChanged { presentation } = event.event else {
            panic!("expected presentation change");
        };
        assert_eq!(
            presentation.ground_grid_origin,
            GroundGridOrigin::WorldOrigin
        );
    }

    #[test]
    fn snapshot_contains_only_logical_viewport_state() {
        let mut app = command_test_app();
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestSnapshot);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("snapshot command must emit a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::Snapshot { state } = event.event else {
            panic!("expected a snapshot event");
        };
        assert_eq!(state.stage.display_name, "fixtures/spinner.usda");
        assert!(state.scene.prims.is_empty());
        assert_eq!(state.selection.target, None);
    }

    #[test]
    fn editor_commands_author_the_live_stage_and_publish_correlation() {
        let mut app = command_test_app();
        let stage = openusd::usd::Stage::builder()
            .in_memory("bridge_editor_test.usda")
            .unwrap();
        stage
            .define_prim("/World")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));

        let define_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::DefinePrim {
                path: "/World/Box".to_owned(),
                type_name: "Cube".to_owned(),
            },
        );
        app.update();

        let define_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("define should publish an event");
        assert_eq!(
            define_event.request_id.as_deref(),
            Some(define_request.as_str())
        );
        assert!(matches!(
            define_event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::DefinePrim,
                ..
            }
        ));

        let attribute_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetAttribute {
                prim_path: "/World/Box".to_owned(),
                name: "size".to_owned(),
                type_name: "double".to_owned(),
                value: serde_json::json!(2.5),
            },
        );
        app.update();
        let attribute_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("attribute should publish an event");
        assert_eq!(
            attribute_event.request_id.as_deref(),
            Some(attribute_request.as_str())
        );
        assert!(matches!(
            attribute_event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SetAttribute,
                ..
            }
        ));

        let live = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage should remain installed");
        assert!(usd_bevy::authoring::prim_exists(&live.stage, "/World/Box"));
        let value = live
            .stage
            .prim(openusd::sdf::path("/World/Box").unwrap())
            .attribute("size")
            .get::<openusd::sdf::Value>()
            .unwrap();
        assert!(
            matches!(value, Some(openusd::sdf::Value::Double(value)) if (value - 2.5).abs() < f64::EPSILON)
        );
    }
}
