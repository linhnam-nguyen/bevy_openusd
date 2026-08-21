use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{
    CameraSource, PROTOCOL_VERSION, SelectionReadModel, ViewportCommand, ViewportEvent,
    ViewportEventEnvelope,
};

use crate::viewport::api::{
    SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand,
    ViewportTreeCommandInbox,
};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::camera::CameraMount;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::scene::SelectedPrim;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};
use super::editor_commands::apply_editor_command;
use super::helpers::{
    emit_presentation_changed, emit_snapshot, reject, set_overlay, timeline_read_model,
};
use super::state::{EditorHistories, RuntimeMutationCoordinator};

/// Applies commands whose state does not require a tree traversal. Tree
/// commands are forwarded to the next system after the scene index refreshes.
/// Stage-authoring commands are delegated to [`apply_editor_command`].
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_viewport_commands(
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
    mut runtime_mutations: ResMut<RuntimeMutationCoordinator>,
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
                runtime_mutations.reset();
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
                    Some(anchor) => match super::helpers::resolve_anchor(&anchor, &scene_index) {
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
                    histories.record(super::state::EditorHistoryDomain::Authoring);
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
                super::helpers::emit_editor_completed(
                    &mut outbox,
                    request_id,
                    viewport_protocol::EditorOperation::SetVariantSelection,
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
            ViewportCommand::SetRendererConfiguration { configuration } => {
                if let Err(error) = configuration.validate() {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                if toggles.renderer != configuration {
                    toggles.renderer = configuration;
                }
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
            // All stage-authoring commands are delegated to editor_commands.
            command => {
                apply_editor_command(
                    command,
                    request_id,
                    &mut outbox,
                    &mut histories,
                    &mut runtime_mutations,
                    stage.as_deref(),
                );
            }
        }
    }
}
