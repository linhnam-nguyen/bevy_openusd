//! Live runtime action driver for benchmark probe scenarios (S1..S24).

use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;
use super::counters::RendererCounters;
use super::scenario::BenchmarkScenarioId;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::input::ViewportNavigationInput;

/// Resource configuring the active scenario action driver.
#[derive(Resource, Debug, Clone)]
pub struct ActiveScenarioDriver {
    pub scenario_id: Option<BenchmarkScenarioId>,
    pub frame_counter: u64,
    pub action_executions: u64,
}

impl Default for ActiveScenarioDriver {
    fn default() -> Self {
        Self {
            scenario_id: None,
            frame_counter: 0,
            action_executions: 0,
        }
    }
}

/// Applies startup configurations for specific scenarios (e.g. S2 grid disabled).
pub fn setup_scenario_driver_system(
    driver: Option<Res<ActiveScenarioDriver>>,
    mut grid: Option<ResMut<GroundGrid>>,
) {
    let Some(driver) = driver else { return };
    let Some(id) = driver.scenario_id else { return };

    if id == BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused {
        if let Some(ref mut grid) = grid {
            grid.visible = false;
        }
    }
}

/// Drives live actions during each frame for dynamic scenarios (S3..S6, S10, S13..S15, S17..S24).
pub fn scenario_action_driver_system(
    driver: Option<ResMut<ActiveScenarioDriver>>,
    mut grid: Option<ResMut<GroundGrid>>,
    mut navigation: Option<ResMut<ViewportNavigationInput>>,
    interface: Option<Res<RenderServerInterface>>,
    mut live_stage: Option<NonSendMut<usd_bevy::LiveStage>>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    let Some(mut driver) = driver else { return };
    let Some(id) = driver.scenario_id else { return };

    driver.frame_counter += 1;
    let frame = driver.frame_counter;

    match id {
        // Steady-state baseline scenarios: no dynamic mutations
        BenchmarkScenarioId::S1NativeHummingbirdGridOnPaused
        | BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused
        | BenchmarkScenarioId::S7NativeVisuallyEmptyLiveStageRetained
        | BenchmarkScenarioId::S8NativeNoLiveStage
        | BenchmarkScenarioId::S9NativeRecoveryIdle
        | BenchmarkScenarioId::S11WebRtcIdleConnected
        | BenchmarkScenarioId::S12WebRtcIdleClientConnected
        | BenchmarkScenarioId::S16WebRtcRemoteVisuallyEmpty => {}

        // Native Camera Orbit / Pan navigation
        BenchmarkScenarioId::S3NativeCameraOrbitPan => {
            if let Some(ref mut nav) = navigation {
                nav.pointer_delta = Vec2::new(
                    (frame as f32 * 0.1).sin() * 4.0,
                    (frame as f32 * 0.1).cos() * 4.0,
                );
                nav.buttons.primary = true;
                nav.focused = true;
                driver.action_executions += 1;
            }
        }

        // Native Grid visibility toggle
        BenchmarkScenarioId::S4NativeGridVisibilityToggle => {
            if frame % 15 == 0 {
                if let Some(ref mut grid) = grid {
                    grid.visible = !grid.visible;
                    driver.action_executions += 1;
                }
            }
        }

        // Native Ground origin change
        BenchmarkScenarioId::S5NativeGroundOriginChange => {
            if frame % 10 == 0 {
                if let Some(ref mut grid) = grid {
                    let new_y = (frame as f32 * 0.1).sin() * 5.0;
                    grid.ground_y = Some(new_y);
                    driver.action_executions += 1;
                }
            }
        }

        // Native Grid style color change
        BenchmarkScenarioId::S6NativeGridStyleColorChange => {
            if frame % 10 == 0 {
                if let Some(ref mut grid) = grid {
                    let r = ((frame as f32 * 0.2).sin() + 1.0) * 0.5;
                    grid.color = Color::srgba(r, 0.38, 0.50, 0.42);
                    driver.action_executions += 1;
                }
            }
        }

        // Native Authoritative USD Change
        BenchmarkScenarioId::S10NativeAuthoritativeUsdChange => {
            if frame == 5 {
                if let Some(ref mut live) = live_stage {
                    let _ = live.stage.define_prim("/Root/BenchmarkMarker");
                    driver.action_executions += 1;
                }
            }
        }

        // WebRTC Remote Grid Visibility Command
        BenchmarkScenarioId::S13WebRtcRemoteGridVisibilityCommand => {
            if frame % 15 == 0 {
                if let Some(ref iface) = interface {
                    let envelope = viewport_protocol::ViewportCommandEnvelope::new(
                        format!("s13-{frame}"),
                        viewport_protocol::ViewportCommand::SetOverlay {
                            overlay: viewport_protocol::OverlayKind::GroundGrid,
                            enabled: frame % 30 < 15,
                        },
                    );
                    let _ = iface.submit_viewport_command(envelope);
                    driver.action_executions += 1;
                }
            }
        }

        // WebRTC Remote Ground Origin Command
        BenchmarkScenarioId::S14WebRtcRemoteGroundOriginCommand => {
            if frame % 10 == 0 {
                if let Some(ref iface) = interface {
                    let origin = if (frame / 10) % 2 == 0 {
                        viewport_protocol::GroundGridOrigin::LoadedScene
                    } else {
                        viewport_protocol::GroundGridOrigin::WorldOrigin
                    };
                    let envelope = viewport_protocol::ViewportCommandEnvelope::new(
                        format!("s14-{frame}"),
                        viewport_protocol::ViewportCommand::SetGroundGridOrigin { origin },
                    );
                    let _ = iface.submit_viewport_command(envelope);
                    driver.action_executions += 1;
                }
            }
        }

        // WebRTC Remote Orbit / Pan input
        BenchmarkScenarioId::S15WebRtcRemoteOrbitPan => {
            if let Some(ref iface) = interface {
                let motion = viewport_protocol::PointerMotion {
                    sequence: frame,
                    dx_css_pixels: (frame as f32 * 0.1).sin() * 4.0,
                    dy_css_pixels: (frame as f32 * 0.1).cos() * 4.0,
                    wheel_x: 0.0,
                    wheel_y: 0.0,
                    viewport_css_width: 1920.0,
                    viewport_css_height: 1080.0,
                    stream_generation: 1,
                };
                let _ = iface.submit_pointer_motion(motion);
                let input = viewport_protocol::InputCommand::ButtonState(
                    viewport_protocol::ButtonState {
                        sequence: frame,
                        buttons: viewport_protocol::PointerButtons {
                            primary: true,
                            ..Default::default()
                        },
                        modifiers: viewport_protocol::InputModifiers::default(),
                        stream_generation: 1,
                    },
                );
                let _ = iface.submit_input(input);
                driver.action_executions += 1;
            }
        }

        // WebRTC Remote Authoritative USD Edit
        BenchmarkScenarioId::S17WebRtcRemoteAuthoritativeUsdEdit => {
            if frame == 5 {
                if let Some(ref iface) = interface {
                    let envelope = viewport_protocol::ViewportCommandEnvelope::new(
                        "s17-edit-1",
                        viewport_protocol::ViewportCommand::SetAttribute {
                            prim_path: "/root/hummingbird".into(),
                            name: "xformOp:translate".into(),
                            type_name: "double3".into(),
                            value: serde_json::json!([1.0, 2.0, 3.0]),
                        },
                    );
                    let _ = iface.submit_viewport_command(envelope);
                    driver.action_executions += 1;
                }
            }
        }

        // WebRTC Remote Command After Long Idle (idle until frame 60)
        BenchmarkScenarioId::S18WebRtcRemoteCommandAfterLongIdle => {
            if frame == 60 {
                if let Some(ref iface) = interface {
                    let envelope = viewport_protocol::ViewportCommandEnvelope::new(
                        "s18-idle-cmd",
                        viewport_protocol::ViewportCommand::SetOverlay {
                            overlay: viewport_protocol::OverlayKind::GroundGrid,
                            enabled: false,
                        },
                    );
                    let _ = iface.submit_viewport_command(envelope);
                    driver.action_executions += 1;
                }
            }
        }

        // S19: Isolation Query Saturation
        BenchmarkScenarioId::S19IsolationQuerySaturation => {
            if let Some(ref mut c) = counters {
                c.query_saturations += 1;
            }
            std::thread::spawn(|| {
                let dummy = (0..1000).fold(0u64, |acc, x| acc.wrapping_add(x));
                std::hint::black_box(dummy);
            });
            driver.action_executions += 1;
        }

        // S20: Isolation Auth Validation Burst
        BenchmarkScenarioId::S20IsolationAuthValidationBurst => {
            if let Some(ref mut c) = counters {
                c.auth_validation_bursts += 1;
                c.auth_lookup_count += 50;
            }
            for _ in 0..50 {
                let _ = blake3::hash(b"bench-token-auth-validation");
            }
            driver.action_executions += 1;
        }

        // S21: Isolation Navigation Under Auth
        BenchmarkScenarioId::S21IsolationNavigationUnderAuth => {
            if let Some(ref mut nav) = navigation {
                nav.pointer_delta = Vec2::new(2.0, 1.0);
                nav.buttons.primary = true;
                nav.focused = true;
            }
            if let Some(ref mut c) = counters {
                c.auth_lookup_count += 20;
            }
            for _ in 0..20 {
                let _ = blake3::hash(b"bench-token-nav-auth");
            }
            driver.action_executions += 1;
        }

        // S22: Isolation Query Command Concurrency
        BenchmarkScenarioId::S22IsolationQueryCommandConcurrency => {
            if let Some(ref mut c) = counters {
                c.query_saturations += 1;
            }
            if let Some(ref iface) = interface {
                let envelope = viewport_protocol::ViewportCommandEnvelope::new(
                    format!("s22-{frame}"),
                    viewport_protocol::ViewportCommand::SetOverlay {
                        overlay: viewport_protocol::OverlayKind::GroundGrid,
                        enabled: true,
                    },
                );
                let _ = iface.submit_viewport_command(envelope);
            }
            driver.action_executions += 1;
        }

        // S23: Isolation Slow/Failing Data Worker
        BenchmarkScenarioId::S23IsolationSlowFailingDataWorker => {
            driver.action_executions += 1;
        }

        // S24: Isolation Auth Revocation Propagation
        BenchmarkScenarioId::S24IsolationAuthRevocationPropagation => {
            if frame % 20 == 0 {
                if let Some(ref mut c) = counters {
                    c.auth_lookup_count += 10;
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
        .add_systems(PreUpdate, scenario_action_driver_system);
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
        app.insert_resource(GroundGrid {
            visible: true,
            ..Default::default()
        });
        app.add_systems(Startup, setup_scenario_driver_system);
        app.update();

        let grid = app.world().resource::<GroundGrid>();
        assert!(!grid.visible);
    }

    #[test]
    fn s4_driver_toggles_grid_visibility() {
        let mut app = App::new();
        app.insert_resource(ActiveScenarioDriver {
            scenario_id: Some(BenchmarkScenarioId::S4NativeGridVisibilityToggle),
            frame_counter: 14,
            action_executions: 0,
        });
        app.insert_resource(GroundGrid {
            visible: true,
            ..Default::default()
        });
        app.add_systems(Update, scenario_action_driver_system);
        app.update();

        let grid = app.world().resource::<GroundGrid>();
        assert!(!grid.visible);
        let driver = app.world().resource::<ActiveScenarioDriver>();
        assert_eq!(driver.action_executions, 1);
    }
}
