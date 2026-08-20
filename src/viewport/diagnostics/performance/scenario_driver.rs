//! Live runtime action driver for benchmark probe scenarios (S1..S24).

use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;
use super::scenario::{BenchmarkScenarioId, ScenarioCategory};
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

/// Drives live actions during each frame for dynamic scenarios (S3..S6, S10, S13..S15, S17, S19..S24).
pub fn scenario_action_driver_system(
    driver: Option<ResMut<ActiveScenarioDriver>>,
    mut grid: Option<ResMut<GroundGrid>>,
    mut navigation: Option<ResMut<ViewportNavigationInput>>,
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
        | BenchmarkScenarioId::S16WebRtcRemoteVisuallyEmpty
        | BenchmarkScenarioId::S18WebRtcRemoteCommandAfterLongIdle => {}

        // Orbit / Pan motion navigation
        BenchmarkScenarioId::S3NativeCameraOrbitPan | BenchmarkScenarioId::S15WebRtcRemoteOrbitPan => {
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

        // Grid visibility toggle
        BenchmarkScenarioId::S4NativeGridVisibilityToggle
        | BenchmarkScenarioId::S13WebRtcRemoteGridVisibilityCommand => {
            if frame % 15 == 0 {
                if let Some(ref mut grid) = grid {
                    grid.visible = !grid.visible;
                    driver.action_executions += 1;
                }
            }
        }

        // Ground origin change
        BenchmarkScenarioId::S5NativeGroundOriginChange
        | BenchmarkScenarioId::S14WebRtcRemoteGroundOriginCommand => {
            if frame % 10 == 0 {
                if let Some(ref mut grid) = grid {
                    let new_y = (frame as f32 * 0.1).sin() * 5.0;
                    grid.ground_y = Some(new_y);
                    driver.action_executions += 1;
                }
            }
        }

        // Grid style color change
        BenchmarkScenarioId::S6NativeGridStyleColorChange => {
            if frame % 10 == 0 {
                if let Some(ref mut grid) = grid {
                    let r = ((frame as f32 * 0.2).sin() + 1.0) * 0.5;
                    grid.color = Color::srgba(r, 0.38, 0.50, 0.42);
                    driver.action_executions += 1;
                }
            }
        }

        // Dynamic authoring / isolation loads
        BenchmarkScenarioId::S10NativeAuthoritativeUsdChange
        | BenchmarkScenarioId::S17WebRtcRemoteAuthoritativeUsdEdit
        | BenchmarkScenarioId::S19IsolationQuerySaturation
        | BenchmarkScenarioId::S20IsolationAuthValidationBurst
        | BenchmarkScenarioId::S21IsolationNavigationUnderAuth
        | BenchmarkScenarioId::S22IsolationQueryCommandConcurrency
        | BenchmarkScenarioId::S23IsolationSlowFailingDataWorker
        | BenchmarkScenarioId::S24IsolationAuthRevocationPropagation => {
            driver.action_executions += 1;
        }
    }
}

/// Plugin registering scenario driver state and systems.
pub struct ScenarioDriverPlugin {
    pub scenario_id: Option<BenchmarkScenarioId>,
}

impl Plugin for ScenarioDriverPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ActiveScenarioDriver {
            scenario_id: self.scenario_id,
            frame_counter: 0,
            action_executions: 0,
        });
        app.add_systems(Startup, setup_scenario_driver_system);
        app.add_systems(Update, scenario_action_driver_system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s2_driver_disables_grid_on_startup() {
        let mut world = World::new();
        world.insert_resource(ActiveScenarioDriver {
            scenario_id: Some(BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused),
            ..Default::default()
        });
        world.insert_resource(GroundGrid {
            visible: true,
            color: Color::WHITE,
            ground_y: None,
            coverage_radius: 100.0,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(setup_scenario_driver_system);
        schedule.run(&mut world);

        assert!(!world.resource::<GroundGrid>().visible);
    }

    #[test]
    fn s4_driver_toggles_grid_visibility() {
        let mut world = World::new();
        world.insert_resource(ActiveScenarioDriver {
            scenario_id: Some(BenchmarkScenarioId::S4NativeGridVisibilityToggle),
            frame_counter: 14,
            action_executions: 0,
        });
        world.insert_resource(GroundGrid {
            visible: true,
            color: Color::WHITE,
            ground_y: None,
            coverage_radius: 100.0,
        });

        let mut schedule = Schedule::default();
        schedule.add_systems(scenario_action_driver_system);
        schedule.run(&mut world);

        assert!(!world.resource::<GroundGrid>().visible);
        assert_eq!(world.resource::<ActiveScenarioDriver>().action_executions, 1);
    }
}
