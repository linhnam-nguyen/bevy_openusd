//! Live runtime action driver for benchmark probe scenarios (S1..S24).

use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;
use std::time::Duration;
use super::counters::RendererCounters;
use super::runner::BenchmarkRunState;
use super::scenario::BenchmarkScenarioId;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::camera::ArcballCameraSet;
use crate::viewport::input::ViewportNavigationInput;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::transport::webrtc::WebRtcTransportState;
use crate::viewport::semantic::SemanticWorkingStore;
use viewport_protocol::{
    ButtonState, GroundGridOrigin, InputCommand, InputModifiers, OverlayKind, PointerButtons,
    PointerMotion, SessionId, SessionRole, ViewportCommand, ViewportCommandEnvelope,
};

/// Resource configuring the active scenario action driver.
#[derive(Resource, Debug, Clone, Default)]
pub struct ActiveScenarioDriver {
    pub scenario_id: Option<BenchmarkScenarioId>,
    pub frame_counter: u64,
    pub action_executions: u64,
}

/// Applies startup configurations for specific scenarios (e.g. S2 grid disabled, session registration).
pub fn setup_scenario_driver_system(
    driver: Option<Res<ActiveScenarioDriver>>,
    mut grid: Option<ResMut<GroundGrid>>,
    mut toggles: Option<ResMut<DisplayToggles>>,
    mut webrtc_state: Option<ResMut<WebRtcTransportState>>,
    semantic_store: Option<Res<SemanticWorkingStore>>,
) {
    let Some(driver) = driver else { return };
    if driver.scenario_id == Some(BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused) {
        if let Some(ref mut toggles) = toggles {
            toggles.show_world_grid = false;
        }
        if let Some(ref mut grid) = grid {
            grid.visible = false;
        }
    }
    if driver.scenario_id == Some(BenchmarkScenarioId::S23IsolationSlowFailingDataWorker) {
        if let Some(store) = semantic_store {
            store.configure_test_mode(Duration::from_millis(100), true);
        }
    }
    if let Some(ref mut state) = webrtc_state {
        if matches!(
            driver.scenario_id,
            Some(
                BenchmarkScenarioId::S21IsolationNavigationUnderAuth
                    | BenchmarkScenarioId::S24IsolationAuthRevocationPropagation
            )
        ) {
            let _ = state
                .sessions
                .register(SessionId::new("bench-controller"), SessionRole::Controller);
        }
        for i in 0..200 {
            let sid = SessionId::new(format!("bench-session-{i}"));
            let _ = state.sessions.register(sid, SessionRole::Observer);
        }
    }
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
    if let Some(ref rs) = run_state {
        if !rs.scene_ready || rs.warmup_frames_remaining > 0 {
            return;
        }
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
        | BenchmarkScenarioId::S16WebRtcRemoteVisuallyEmpty => {}

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
            if frame % 15 == 0 {
                if let Some(ref mut t) = toggles {
                    t.show_world_grid = !t.show_world_grid;
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S5NativeGroundOriginChange => {
            if frame % 10 == 0 {
                if let Some(ref mut t) = toggles {
                    t.ground_grid_origin = if (frame / 10) % 2 == 0 {
                        GroundGridOrigin::LoadedScene
                    } else {
                        GroundGridOrigin::WorldOrigin
                    };
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S6NativeGridStyleColorChange => {
            if frame % 10 == 0 {
                if let Some(ref mut g) = grid {
                    let r = ((frame as f32 * 0.2).sin() + 1.0) * 0.5;
                    g.color = Color::srgba(r, 0.38, 0.50, 0.42);
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S10NativeAuthoritativeUsdChange => {
            if frame == 5 {
                if let Some(ref mut live) = live_stage {
                    let _ = live.stage.define_prim("/Root/BenchmarkMarker");
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S13WebRtcRemoteGridVisibilityCommand => {
            if frame % 15 == 0 {
                if let Some(ref iface) = interface {
                    let cmd = ViewportCommand::SetOverlay {
                        overlay: OverlayKind::GroundGrid,
                        enabled: frame % 30 < 15,
                    };
                    let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(format!("s13-{frame}"), cmd));
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S14WebRtcRemoteGroundOriginCommand => {
            if frame % 10 == 0 {
                if let Some(ref iface) = interface {
                    let origin = if (frame / 10) % 2 == 0 {
                        GroundGridOrigin::LoadedScene
                    } else {
                        GroundGridOrigin::WorldOrigin
                    };
                    let cmd = ViewportCommand::SetGroundGridOrigin { origin };
                    let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(format!("s14-{frame}"), cmd));
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S15WebRtcRemoteOrbitPan => {
            if let Some(ref iface) = interface {
                let motion = PointerMotion {
                    sequence: frame,
                    dx_css_pixels: (frame as f32 * 0.05).sin() * 4.0,
                    dy_css_pixels: (frame as f32 * 0.05).cos() * 3.0,
                    wheel_x: 0.0,
                    wheel_y: if frame % 30 == 0 { 120.0 } else { 0.0 },
                    viewport_css_width: 1920.0,
                    viewport_css_height: 1080.0,
                    stream_generation: 1,
                };
                let _ = iface.submit_pointer_motion(motion);
                let btn = ButtonState {
                    sequence: frame,
                    buttons: PointerButtons { primary: true, secondary: true, auxiliary: false },
                    modifiers: InputModifiers::default(),
                    stream_generation: 1,
                };
                let _ = iface.submit_input(InputCommand::ButtonState(btn));
                driver.action_executions += 1;
            }
        }
        BenchmarkScenarioId::S17WebRtcRemoteAuthoritativeUsdEdit => {
            if frame == 5 {
                if let Some(ref iface) = interface {
                    let cmd = ViewportCommand::SetAttribute {
                        prim_path: "/root/hummingbird".into(),
                        name: "xformOp:translate".into(),
                        type_name: "double3".into(),
                        value: serde_json::json!([1.0, 2.0, 3.0]),
                    };
                    let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new("s17-edit-1", cmd));
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S18WebRtcRemoteCommandAfterLongIdle => {
            if frame == 60 {
                if let Some(ref iface) = interface {
                    let cmd = ViewportCommand::SetOverlay {
                        overlay: OverlayKind::GroundGrid,
                        enabled: false,
                    };
                    let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new("s18-idle-cmd", cmd));
                    driver.action_executions += 1;
                }
            }
        }
        BenchmarkScenarioId::S19IsolationQuerySaturation => {
            if let Some(ref iface) = interface {
                let cmd = ViewportCommand::SearchScene { query: "root".into(), offset: 0, limit: 50 };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(format!("s19-q-{frame}"), cmd));
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
                let q = ViewportCommand::SearchScene { query: "hummingbird".into(), offset: 0, limit: 20 };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(format!("s22-q-{frame}"), q));
                let cmd = ViewportCommand::SetOverlay { overlay: OverlayKind::GroundGrid, enabled: true };
                let _ = iface.submit_viewport_command(ViewportCommandEnvelope::new(format!("s22-c-{frame}"), cmd));
            }
            driver.action_executions += 1;
        }
        BenchmarkScenarioId::S23IsolationSlowFailingDataWorker => {
            if let Some(ref iface) = interface {
                let cmd = ViewportCommand::SearchScene {
                    query: "root".into(),
                    offset: 0,
                    limit: 20,
                };
                let _ = iface.submit_viewport_command(
                    ViewportCommandEnvelope::new(format!("s23-q-{frame}"), cmd),
                );
            }
            driver.action_executions += 1;
        }
        BenchmarkScenarioId::S24IsolationAuthRevocationPropagation => {
            if frame == 20 {
                let controller = SessionId::new("bench-controller");
                let revoked = webrtc_state
                    .as_mut()
                    .is_some_and(|state| state.sessions.unregister(&controller));
                if revoked {
                    let rejected_after_revoke = if let (Some(iface), Some(state)) =
                        (&interface, &webrtc_state)
                    {
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
                    {
                        if let Some(ref mut c) = counters {
                            c.auth_failures += 1;
                        }
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
            scenario_action_driver_system
                .after(ArcballCameraSet::PrepareInput)
                .before(ArcballCameraSet::ApplyInput),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s2_driver_disables_grid_on_startup() {
        let mut app = App::new();
        app.insert_resource(ActiveScenarioDriver {
            scenario_id: Some(BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused),
            frame_counter: 0,
            action_executions: 0,
        });
        app.insert_resource(GroundGrid { visible: true, ..Default::default() });
        app.insert_resource(DisplayToggles { show_world_grid: true, ..Default::default() });
        app.add_systems(Startup, setup_scenario_driver_system);
        app.update();

        assert!(!app.world().resource::<GroundGrid>().visible);
        assert!(!app.world().resource::<DisplayToggles>().show_world_grid);
    }

    #[test]
    fn s4_driver_toggles_grid_visibility() {
        let mut app = App::new();
        app.insert_resource(ActiveScenarioDriver {
            scenario_id: Some(BenchmarkScenarioId::S4NativeGridVisibilityToggle),
            frame_counter: 14,
            action_executions: 0,
        });
        app.insert_resource(BenchmarkRunState {
            scene_ready: true,
            warmup_frames_remaining: 0,
            target_frames_remaining: 120,
            samples: vec![],
            is_completed: false,
        });
        app.insert_resource(GroundGrid { visible: true, ..Default::default() });
        app.insert_resource(DisplayToggles { show_world_grid: true, ..Default::default() });
        app.add_systems(Update, scenario_action_driver_system);
        app.update();

        assert!(!app.world().resource::<DisplayToggles>().show_world_grid);
        assert_eq!(app.world().resource::<ActiveScenarioDriver>().action_executions, 1);
    }
}
