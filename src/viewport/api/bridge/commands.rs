use bevy::prelude::*;
use viewport_protocol::{PROTOCOL_VERSION, ViewportCommand};

use super::editor_commands::apply_editor_command;
use super::helpers::{
    emit_presentation_changed, emit_snapshot, emit_viewer_settings_changed, reject,
};
use super::state::EditorHistories;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand};
use crate::viewport::rendering::sampling::{
    ActiveUpscaler, SamplingCapabilities, SamplingSelectionError, choose_upscaler,
};
mod cadence;
mod camera;
mod presentation;
mod selection;
mod selection_dispatch;
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

        let Some((command, request_id)) = selection_dispatch::apply_selection_command(
            envelope.command,
            request_id,
            &mut outbox,
            &mut state.selected_prim,
            &mut state.selected_targets,
            &state.scene_index,
        ) else {
            continue;
        };

        match command {
            ViewportCommand::RequestSnapshot => {
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &state.configuration.p0(),
                    &state.spawned,
                    &state.selected_targets.0,
                    state.selected_targets.revision(),
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.camera_orientation.latest,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
            }
            ViewportCommand::RequestSceneChildren { .. }
            | ViewportCommand::SearchScene { .. }
            | ViewportCommand::SearchBim { .. }
            | ViewportCommand::RequestBimProperties
            | ViewportCommand::RequestBimPropertyProvenance { .. }
            | ViewportCommand::RequestHierarchyChildren { .. }
            | ViewportCommand::SearchHierarchy { .. }
            | ViewportCommand::SetHierarchySource { .. } => {
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
                    state.selected_targets.revision(),
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.camera_orientation.latest,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
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
                    let suppression = stage.mark_authored_guard(prim_path.clone());
                    if let Err(error) = state.histories.authoring.set_variant(
                        &stage.stage,
                        &prim_path,
                        &set_name,
                        &option,
                    ) {
                        reject(&mut outbox, request_id, error.to_string());
                        continue;
                    }
                    suppression.commit();
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
                    state.selected_targets.revision(),
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.camera_orientation.latest,
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
                    state.selected_targets.revision(),
                    &state.viewer_settings.0,
                    &state.scene_index,
                    &state.camera_mount,
                    &state.camera_orientation.latest,
                    &state.clock,
                    &state.toggles,
                    &state.tuning,
                    state.physics.0,
                );
            }
            ViewportCommand::SetCameraSource { source } => {
                camera::set_camera_source(request_id, source, &mut outbox, &mut state.camera_mount);
            }
            ViewportCommand::SetStandardView { view } => {
                camera::set_standard_view(
                    request_id,
                    view,
                    &mut outbox,
                    &mut state.camera_mount,
                    &mut state.fly_to,
                    &state.cameras,
                );
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
                if configuration.render_mode == viewport_protocol::RenderMode::RayTraced
                    && !state
                        .solari
                        .as_ref()
                        .is_some_and(|capability| capability.supported())
                {
                    reject(
                        &mut outbox,
                        request_id,
                        "ray traced rendering is unsupported by the active Solari capability"
                            .to_owned(),
                    );
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
                if state.viewer_settings.0.environment != settings {
                    state.viewer_settings.0.environment = settings;
                }
                emit_viewer_settings_changed(&mut outbox, request_id, &state.viewer_settings.0);
            }
            ViewportCommand::SetSamplingPreference { preference } => {
                // FSR is a forward-compatible provider vocabulary only. No
                // reviewed implementation is active in B4, so sampling On
                // may select DLSS or reject explicitly.
                let capabilities = SamplingCapabilities::new(state.dlss.supported(), false);
                let active = match choose_upscaler(preference.enabled, capabilities) {
                    Ok(active) => active,
                    Err(SamplingSelectionError::NoProviderAvailable) => {
                        reject(
                            &mut outbox,
                            request_id,
                            "sampling is unsupported by the active renderer providers".to_owned(),
                        );
                        continue;
                    }
                };
                state.sampling.apply(preference.enabled, active);
                state.dlss_camera.enabled = active == ActiveUpscaler::Dlss;
                state
                    .viewer_settings
                    .set_sampling(preference.enabled, active.provider());
                emit_viewer_settings_changed(&mut outbox, request_id, &state.viewer_settings.0);
            }
            ViewportCommand::SetSelectionPresentationSettings { settings } => {
                state.viewer_settings.0.selection = settings;
                emit_viewer_settings_changed(&mut outbox, request_id, &state.viewer_settings.0);
            }
            ViewportCommand::SetClassificationColorPlan { intent } => {
                let Some(plan) = state.classification_color_plan.as_deref_mut() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "classification color presentation is unavailable".to_owned(),
                    );
                    continue;
                };
                if let Err(error) = plan.accept_intent(intent) {
                    reject(&mut outbox, request_id, error.to_owned());
                }
            }
            ViewportCommand::SetSectionBox { enabled } => {
                state.viewer_settings.set_section_box_enabled(enabled);
                emit_viewer_settings_changed(&mut outbox, request_id, &state.viewer_settings.0);
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
                    state
                        .semantic
                        .as_ref()
                        .and_then(|semantic| semantic.snapshot()),
                    state.stage.as_deref(),
                    state
                        .stage_handle
                        .as_ref()
                        .map(|handle| handle.path.as_path()),
                    &state.selected_targets,
                );
            }
        }
    }
}
