use super::{ActiveScenarioDriver, BenchmarkScenarioId};
use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::SelectedTargets;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::semantic::SemanticWorkingStore;
use crate::viewport::transport::webrtc::WebRtcTransportState;
use bevy::prelude::{Res, ResMut};
use bevy_glacial::prelude::GroundGrid;
use std::time::Duration;
use viewport_protocol::{SelectionReadModel, SessionId, SessionRole};

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
    if driver.scenario_id == Some(BenchmarkScenarioId::S23IsolationSlowFailingDataWorker)
        && let Some(store) = semantic_store
    {
        store.configure_test_mode(Duration::from_millis(100), true);
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

/// Enables a deliberately opt-in renderer smoke that exercises the Section
/// Box material and prepass route on a loaded benchmark scene. Normal
/// launches never select or enable Section Box through this harness.
pub(in crate::viewport) fn setup_section_box_smoke_system(
    driver: Option<Res<ActiveScenarioDriver>>,
    scene_index: Option<Res<SceneAnchorIndex>>,
    selection: Option<ResMut<SelectedTargets>>,
    settings: Option<ResMut<ViewerSettingsState>>,
) {
    if std::env::var_os("USDHUB_SECTION_BOX_SMOKE").is_none()
        || driver
            .as_ref()
            .is_none_or(|driver| driver.scenario_id.is_none())
    {
        return;
    }
    let (Some(scene_index), Some(mut selection), Some(mut settings)) =
        (scene_index, selection, settings)
    else {
        return;
    };
    let Some(anchor) = scene_index
        .roots_read_model()
        .prims
        .first()
        .map(|node| node.anchor.clone())
    else {
        return;
    };
    if selection.0.targets.is_empty() {
        let _ = selection.replace(SelectionReadModel {
            targets: vec![anchor.clone()],
            primary: Some(anchor),
        });
    }
    if !settings.section_box_enabled() {
        settings.set_section_box_enabled(true);
    }
}
