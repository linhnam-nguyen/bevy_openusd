//! Live runtime action driver for benchmark probe scenarios (S1..S24).

use super::counters::RendererCounters;
use super::runner::BenchmarkRunState;
use super::scenario::BenchmarkScenarioId;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::camera::ArcballCameraSet;
use crate::viewport::input::ViewportNavigationInput;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::transport::webrtc::WebRtcTransportState;
use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;
use viewport_protocol::{
    ButtonState, GroundGridOrigin, InputCommand, InputModifiers, OverlayKind, PointerButtons,
    PointerMotion, SessionId, ViewportCommand, ViewportCommandEnvelope,
};

#[path = "scenario_driver_setup.rs"]
mod setup;
pub use setup::setup_scenario_driver_system;
pub(crate) use setup::setup_section_box_smoke_system;

/// Resource configuring the active scenario action driver.
#[derive(Resource, Debug, Clone, Default)]
pub struct ActiveScenarioDriver {
    pub scenario_id: Option<BenchmarkScenarioId>,
    pub frame_counter: u64,
    pub action_executions: u64,
}

/// Drives live actions during each frame for dynamic scenarios (S3..S6, S10, S13..S15, S17..S24).
pub fn scenario_action_driver_system(
    driver: Option<ResMut<ActiveScenarioDriver>>,
    run_state: Option<Res<BenchmarkRunState>>,
    mut toggles: Option<ResMut<DisplayToggles>>,
    mut grid: Option<ResMut<GroundGrid>>,
    mut navigation: Option<ResMut<ViewportNavigationInput>>,
    interface: Option<Res<RenderServerInterface>>,
    mut live_stage: Option<NonSendMut<usd_bevy::LiveStage>>,
    mut webrtc_state: Option<ResMut<WebRtcTransportState>>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    let Some(mut driver) = driver else { return };
    let Some(id) = driver.scenario_id else { return };

    // Synchronize scenario action triggers strictly to the post-warmup measurement window
    if let Some(ref rs) = run_state
        && (!rs.scene_ready || rs.warmup_frames_remaining > 0)
    {
        return;
    }

    driver.frame_counter += 1;
    let frame = driver.frame_counter;

    match id {
        BenchmarkScenarioId::S1NativeHummingbirdGridOnPaused
        | BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused
        | BenchmarkScenarioId::S7NativeVisuallyEmptyLiveStageRetained
        | BenchmarkScenarioId::S8NativeNoLiveStage
        | BenchmarkScenarioId::S9NativeRecoveryIdle
        | BenchmarkScenarioId::S11WebRtcIdleConnected
        | BenchmarkScenarioId::S12WebRtcIdleClientConnected
        | BenchmarkScenarioId::S16WebRtcRemoteVisuallyEmpty
        | BenchmarkScenarioId::S13WebRtcRemoteGridVisibilityCommand
        | BenchmarkScenarioId::S14WebRtcRemoteGroundOriginCommand
        | BenchmarkScenarioId::S15WebRtcRemoteOrbitPan
        | BenchmarkScenarioId::S17WebRtcRemoteAuthoritativeUsdEdit
        | BenchmarkScenarioId::S18WebRtcRemoteCommandAfterLongIdle => {
            // S12-S18 are real-client scenarios. Their UsdHubUI harness owns
            // the action and the WebRTC round trip; the benchmark server only
            // observes the resulting authoritative state and renderer metrics.
        }

        BenchmarkScenarioId::S3NativeCameraOrbitPan => {
            if let Some(ref mut nav) = navigation {
                let a = (frame as f32) * 0.05;
                nav.pointer_delta = Vec2::new(a.sin() * 5.0, a.cos() * 3.0);
                nav.buttons.primary = true;
                nav.buttons.secondary = true;
                nav.focused = true;
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S4NativeGridVisibilityToggle => {
            if frame % 15 == 0
                && let Some(ref mut t) = toggles
            {
                t.renderer.grid = !t.renderer.grid;
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S5NativeGroundOriginChange => {
            if frame % 10 == 0
                && let Some(ref mut t) = toggles
            {
                t.ground_grid_origin = if (frame / 10) % 2 == 0 {
                    GroundGridOrigin::LoadedScene
                } else {
                    GroundGridOrigin::WorldOrigin
                };
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S6NativeGridStyleColorChange => {
            if frame % 10 == 0
                && let Some(ref mut g) = grid
            {
                let r = ((frame as f32 * 0.2).sin() + 1.0) * 0.5;
                g.color = Color::srgba(r, 0.38, 0.50, 0.42);
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S10NativeAuthoritativeUsdChange => {
            if frame == 5
                && let Some(ref mut live) = live_stage
            {
                let _ = live.stage.define_prim("/Root/BenchmarkMarker");
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S19IsolationQuerySaturation => {
            if let Some(ref iface) = interface {
                let cmd = ViewportCommand::SearchScene {
                    query: "root".into(),
                    offset: 0,
                    limit: 50,
                };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(
                    format!("s19-q-{frame}"),
                    cmd,
                ));
            }
            driver.action_executions += 1;
        }
        BenchmarkScenarioId::S20IsolationAuthValidationBurst => {
            let mut validated = 0u64;
            if let Some(ref state) = webrtc_state {
                for i in 0..50 {
                    let sid = SessionId::new(format!("bench-session-{}", (frame * 50 + i) % 200));
                    if state.sessions.role(&sid).is_some() {
                        validated += 1;
                    }
                }
            }
            if let Some(ref mut c) = counters {
                c.auth_validation_bursts += 1;
                c.auth_validations += validated;
                c.auth_snapshot_hits += validated;
                c.auth_high_water = c.auth_high_water.max(validated);
            }
            driver.action_executions += 1;
        }
        BenchmarkScenarioId::S21IsolationNavigationUnderAuth => {
            let controller = SessionId::new("bench-controller");
            let mut validations = 0u64;
            let mut accepted = 0u64;
            if let (Some(iface), Some(state)) = (&interface, &webrtc_state) {
                for i in 0..20 {
                    let sequence = frame.saturating_mul(40).saturating_add(i * 2 + 1);
                    let motion = PointerMotion {
                        sequence,
                        x_css_pixels: 960.0,
                        y_css_pixels: 540.0,
                        dx_css_pixels: 2.0,
                        dy_css_pixels: 1.0,
                        wheel_x: 0.0,
                        wheel_y: if i == 0 { 120.0 } else { 0.0 },
                        viewport_css_width: 1920.0,
                        viewport_css_height: 1080.0,
                        stream_generation: 1,
                    };
                    let button = ButtonState {
                        sequence: sequence + 1,
                        buttons: PointerButtons {
                            primary: true,
                            secondary: true,
                            auxiliary: false,
                        },
                        modifiers: InputModifiers::default(),
                        stream_generation: 1,
                    };
                    for command in [
                        InputCommand::PointerMotion(motion),
                        InputCommand::ButtonState(button),
                    ] {
                        validations += 1;
                        if state.sessions.role(&controller).is_some()
                            && state
                                .submit_authenticated_input(&controller, iface, command)
                                .is_ok()
                        {
                            accepted += 1;
                        }
                    }
                }
            }
            if let Some(ref mut c) = counters {
                c.auth_validations += validations;
                c.auth_snapshot_hits += validations;
                c.auth_high_water = c.auth_high_water.max(validations);
            }
            if accepted > 0 {
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S22IsolationQueryCommandConcurrency => {
            if let Some(ref iface) = interface {
                let q = ViewportCommand::SearchScene {
                    query: "hummingbird".into(),
                    offset: 0,
                    limit: 20,
                };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(
                    format!("s22-q-{frame}"),
                    q,
                ));
                let cmd = ViewportCommand::SetOverlay {
                    overlay: OverlayKind::GroundGrid,
                    enabled: true,
                };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(
                    format!("s22-c-{frame}"),
                    cmd,
                ));
            }
            driver.action_executions += 1;
        }
        BenchmarkScenarioId::S23IsolationSlowFailingDataWorker => {
            // Keep the first failing request current until its delayed worker
            // response returns. Only then start the query burst; otherwise
            // latest-query replacement removes the probe's correlation entry
            // before the failure can reach the normal requester bridge.
            let failure_probe = frame == 1;
            let backlog_burst = (16..=36).contains(&frame);
            if (failure_probe || backlog_burst)
                && let Some(ref iface) = interface
            {
                let cmd = ViewportCommand::SearchScene {
                    query: "root".into(),
                    offset: 0,
                    limit: 20,
                };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(
                    if failure_probe {
                        "s23-failure-probe".into()
                    } else {
                        format!("s23-burst-q-{frame}")
                    },
                    cmd,
                ));
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S24IsolationAuthRevocationPropagation => {
            if frame == 20 {
                let controller = SessionId::new("bench-controller");
                let revoked = webrtc_state
                    .as_mut()
                    .is_some_and(|state| state.sessions.unregister(&controller));
                if revoked {
                    let rejected_after_revoke =
                        if let (Some(iface), Some(state)) = (&interface, &webrtc_state) {
                            state
                                .submit_authenticated_input(
                                    &controller,
                                    iface,
                                    InputCommand::ButtonState(ButtonState {
                                        sequence: frame,
                                        buttons: PointerButtons::default(),
                                        modifiers: InputModifiers::default(),
                                        stream_generation: 1,
                                    }),
                                )
                                .is_err()
                        } else {
                            false
                        };
                    if rejected_after_revoke
                        && webrtc_state
                            .as_ref()
                            .is_some_and(|state| state.sessions.role(&controller).is_none())
                        && let Some(ref mut c) = counters
                    {
                        c.auth_failures += 1;
                    }
                }
            }
            driver.action_executions += 1;
        }
    }
}

/// Plugin registering the benchmark scenario action driver systems.
pub struct ScenarioDriverPlugin {
    pub scenario_id: Option<BenchmarkScenarioId>,
}

impl Plugin for ScenarioDriverPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ActiveScenarioDriver {
            scenario_id: self.scenario_id,
            frame_counter: 0,
            action_executions: 0,
        })
        .add_systems(Startup, setup_scenario_driver_system)
        .add_systems(
            Update,
            setup_section_box_smoke_system
                .after(crate::viewport::api::ViewportBridgeSet::ApplyCommands)
                .before(crate::viewport::scene::sync_section_box_state),
        )
        .add_systems(
            Update,
            scenario_action_driver_system
                .after(ArcballCameraSet::PrepareInput)
                .before(ArcballCameraSet::ApplyInput),
        );
    }
}

#[cfg(test)]
#[path = "scenario_driver_tests.rs"]
mod tests;
