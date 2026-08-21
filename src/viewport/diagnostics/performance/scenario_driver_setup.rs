use super::{ActiveScenarioDriver, BenchmarkScenarioId};
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::semantic::SemanticWorkingStore;
use crate::viewport::transport::webrtc::WebRtcTransportState;
use bevy::prelude::{Res, ResMut};
use bevy_glacial::prelude::GroundGrid;
use std::time::Duration;
use viewport_protocol::{SessionId, SessionRole};

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
            toggles.renderer.grid = false;
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
