use bevy::prelude::*;
use viewport_protocol::{PROTOCOL_VERSION, ViewportCommand};

use super::editor_commands::apply_editor_command;
use super::helpers::{
    emit_presentation_changed, emit_snapshot, emit_viewer_settings_changed, reject,
};
use super::state::EditorHistories;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand};
mod cadence;
mod camera;
mod presentation;
mod selection;
mod state;
mod timeline;

pub(super) use cadence::apply_pending_renderer_cadence;
use state::ApplyViewportCommandState;

/// Applies commands whose state does not require a tree traversal. Tree
/// commands are forwarded to the next system after the scene index refreshes.
/// Stage-authoring commands are delegated to [`apply_editor_command`].
pub(super) fn apply_viewport_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut state: ApplyViewportCommandState<'_, '_>,
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
            ViewportCommand::RequestSnapshot => {
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &state.configuration.p0(),
                    &state.spawned,
                    &state.selected_targets.0,
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
            }
            ViewportCommand::RequestSceneChildren { .. } | ViewportCommand::SearchScene { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "scene query command was not dispatched".to_owned(),
                );
            }
            ViewportCommand::ReloadSession => {
                state.reload.requested = true;
                *state.histories = EditorHistories::default();
                state.runtime_mutations.reset();
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &state.configuration.p0(),
                    &state.spawned,
                    &state.selected_targets.0,
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
            }
            ViewportCommand::SelectTarget { target } => {
                selection::select_target(
                    request_id,
                    target,
                    &mut outbox,
                    &mut state.selected_prim,
                    &mut state.selected_targets,
                    &state.scene_index,
                );
            }
            ViewportCommand::ReplaceSelection { targets, primary } => {
                selection::replace_selection(
                    request_id,
                    targets,
                    primary,
                    &mut outbox,
                    &mut state.selected_prim,
                    &mut state.selected_targets,
                    &state.scene_index,
                );
            }
            ViewportCommand::AddSelectionTarget {
                target,
                make_primary,
            } => {
                selection::add_selection_target(
                    request_id,
                    target,
                    make_primary,
                    &mut outbox,
                    &mut state.selected_prim,
                    &mut state.selected_targets,
                    &state.scene_index,
                );
            }
            ViewportCommand::RemoveSelectionTarget { target } => {
                selection::remove_selection_target(
                    request_id,
                    target,
                    &mut outbox,
                    &mut state.selected_prim,
                    &mut state.selected_targets,
                    &state.scene_index,
                );
            }
            ViewportCommand::FocusTarget { target, mode } => {
                state.tree_commands.push(ViewportTreeCommand::Focus {
                    request_id,
                    target,
                    mode,
                });
            }
            ViewportCommand::SetSubtreeVisibility { target, visible } => {
                state
                    .tree_commands
                    .push(ViewportTreeCommand::SetSubtreeVisibility {
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
                state
                    .tuning
                    .variants
                    .insert((prim_path.clone(), set_name.clone()), option.clone());
                if let Some(stage) = state.stage.as_deref() {
                    stage.mark_authored(prim_path.clone());
                    if let Err(error) = state.histories.authoring.set_variant(
                        &stage.stage,
                        &prim_path,
                        &set_name,
                        &option,
                    ) {
                        reject(&mut outbox, request_id, error.to_string());
                        continue;
                    }
                    state
                        .histories
                        .record(super::state::EditorHistoryDomain::Authoring);
                }
                emit_snapshot(
                    &mut outbox,
                    request_id.clone(),
                    &state.configuration.p0(),
                    &state.spawned,
                    &state.selected_targets.0,
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
                super::helpers::emit_editor_completed(
                    &mut outbox,
                    request_id,
                    viewport_protocol::EditorOperation::SetVariantSelection,
                    vec![format!("{prim_path}.{set_name}")],
                    &state.histories,
                );
            }
            ViewportCommand::ResetVariantSelection {
                prim_path,
                set_name,
            } => {
                state
                    .tuning
                    .variants
                    .remove(&(prim_path.clone(), set_name.clone()));
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &state.configuration.p0(),
                    &state.spawned,
                    &state.selected_targets.0,
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
            }
            ViewportCommand::SetCameraSource { source } => {
                camera::set_camera_source(request_id, source, &mut outbox, &mut state.camera_mount);
            }
            ViewportCommand::SetPlayback { playing } => {
                timeline::set_playback(request_id, playing, &mut outbox, &mut state.clock);
            }
            ViewportCommand::Seek { seconds } => {
                timeline::seek(request_id, seconds, &mut outbox, &mut state.clock);
            }
            ViewportCommand::SetOverlay { overlay, enabled } => {
                presentation::set_overlay_command(
                    request_id,
                    overlay,
                    enabled,
                    &mut outbox,
                    &mut state.toggles,
                    &state.tuning,
                );
            }
            ViewportCommand::SetGroundGridOrigin { origin } => {
                presentation::set_grid_origin(
                    request_id,
                    origin,
                    &mut outbox,
                    &mut state.toggles,
                    &state.tuning,
                );
            }
            ViewportCommand::SetRendererConfiguration { configuration } => {
                if let Err(error) = configuration.validate() {
                    reject(&mut outbox, request_id, error.to_string());
                    continue;
                }
                let fps_change_pending = if let Some(cadence) =
                    state.configuration.p1().as_deref_mut()
                {
                    let pending =
                        cadence.request_local(configuration.preferred_fps, request_id.clone());
                    state.toggles.renderer = configuration;
                    state.toggles.renderer.preferred_fps = cadence.effective_renderer_target_fps();
                    pending
                } else {
                    state.toggles.renderer = configuration;
                    false
                };
                if !fps_change_pending {
                    emit_presentation_changed(
                        &mut outbox,
                        request_id,
                        &state.toggles,
                        &state.tuning,
                    );
                }
            }
            ViewportCommand::SetEnvironmentSettings { settings } => {
                state.viewer_settings.0.environment = settings;
                emit_viewer_settings_changed(&mut outbox, request_id, &state.viewer_settings.0);
            }
            ViewportCommand::SetSamplingPreference { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "sampling preference is not applied in this milestone".to_owned(),
                );
            }
            ViewportCommand::SetSelectionPresentationSettings { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "selection presentation settings are not applied in this milestone".to_owned(),
                );
            }
            ViewportCommand::SetSectionBox { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "section box is not applied in this milestone".to_owned(),
                );
            }
            ViewportCommand::SetPrimMarkerBias { bias } => {
                presentation::set_prim_marker_bias(
                    request_id,
                    bias,
                    &mut outbox,
                    &mut state.toggles,
                    &state.tuning,
                );
            }
            ViewportCommand::SetLightIntensity { scale } => {
                presentation::set_light_intensity(
                    request_id,
                    scale,
                    &mut outbox,
                    &mut state.toggles,
                    &state.tuning,
                );
            }
            ViewportCommand::SetCurveTuning { tuning: next } => {
                presentation::set_curve_tuning(
                    request_id,
                    next,
                    &mut outbox,
                    &mut state.toggles,
                    &mut state.tuning,
                );
            }
            ViewportCommand::SetPhysicsRunning { running } => {
                presentation::set_physics(request_id, running, &mut outbox, &mut state.physics);
            }
            // All stage-authoring commands are delegated to editor_commands.
            command => {
                apply_editor_command(
                    command,
                    request_id,
                    &mut outbox,
                    &mut state.histories,
                    &mut state.runtime_mutations,
                    state.stage.as_deref(),
                );
            }
        }
    }
}
