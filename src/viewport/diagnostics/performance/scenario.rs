//! Scenario definitions and invariant steady-state expectations for rendering benchmarks (S1..S24).

use serde::{Deserialize, Serialize};

#[path = "scenario_definitions.rs"]
mod scenario_definitions;

/// Canonical Benchmark Scenario Identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BenchmarkScenarioId {
    // Native steady-state & presentation regression probes
    S1NativeHummingbirdGridOnPaused,
    S2NativeHummingbirdGridOffPaused,
    S3NativeCameraOrbitPan,
    S4NativeGridVisibilityToggle,
    S5NativeGroundOriginChange,
    S6NativeGridStyleColorChange,
    S7NativeVisuallyEmptyLiveStageRetained,
    S8NativeNoLiveStage,
    S9NativeRecoveryIdle,
    S10NativeAuthoritativeUsdChange,

    // Headless WebRTC server & remote client path
    S11WebRtcIdleConnected,
    S12WebRtcIdleClientConnected,
    S13WebRtcRemoteGridVisibilityCommand,
    S14WebRtcRemoteGroundOriginCommand,
    S15WebRtcRemoteOrbitPan,
    S16WebRtcRemoteVisuallyEmpty,
    S17WebRtcRemoteAuthoritativeUsdEdit,
    S18WebRtcRemoteCommandAfterLongIdle,

    // Render / Data Plane Isolation
    S19IsolationQuerySaturation,
    S20IsolationAuthValidationBurst,
    S21IsolationNavigationUnderAuth,
    S22IsolationQueryCommandConcurrency,
    S23IsolationSlowFailingDataWorker,
    S24IsolationAuthRevocationPropagation,
}

impl BenchmarkScenarioId {
    /// Return the standardized short code (e.g. "S1", "S14").
    pub fn code(&self) -> &'static str {
        match self {
            Self::S1NativeHummingbirdGridOnPaused => "S1",
            Self::S2NativeHummingbirdGridOffPaused => "S2",
            Self::S3NativeCameraOrbitPan => "S3",
            Self::S4NativeGridVisibilityToggle => "S4",
            Self::S5NativeGroundOriginChange => "S5",
            Self::S6NativeGridStyleColorChange => "S6",
            Self::S7NativeVisuallyEmptyLiveStageRetained => "S7",
            Self::S8NativeNoLiveStage => "S8",
            Self::S9NativeRecoveryIdle => "S9",
            Self::S10NativeAuthoritativeUsdChange => "S10",
            Self::S11WebRtcIdleConnected => "S11",
            Self::S12WebRtcIdleClientConnected => "S12",
            Self::S13WebRtcRemoteGridVisibilityCommand => "S13",
            Self::S14WebRtcRemoteGroundOriginCommand => "S14",
            Self::S15WebRtcRemoteOrbitPan => "S15",
            Self::S16WebRtcRemoteVisuallyEmpty => "S16",
            Self::S17WebRtcRemoteAuthoritativeUsdEdit => "S17",
            Self::S18WebRtcRemoteCommandAfterLongIdle => "S18",
            Self::S19IsolationQuerySaturation => "S19",
            Self::S20IsolationAuthValidationBurst => "S20",
            Self::S21IsolationNavigationUnderAuth => "S21",
            Self::S22IsolationQueryCommandConcurrency => "S22",
            Self::S23IsolationSlowFailingDataWorker => "S23",
            Self::S24IsolationAuthRevocationPropagation => "S24",
        }
    }

    /// Return the category of the scenario.
    pub fn category(&self) -> ScenarioCategory {
        match self {
            Self::S1NativeHummingbirdGridOnPaused
            | Self::S2NativeHummingbirdGridOffPaused
            | Self::S3NativeCameraOrbitPan
            | Self::S4NativeGridVisibilityToggle
            | Self::S5NativeGroundOriginChange
            | Self::S6NativeGridStyleColorChange
            | Self::S7NativeVisuallyEmptyLiveStageRetained
            | Self::S8NativeNoLiveStage
            | Self::S9NativeRecoveryIdle
            | Self::S10NativeAuthoritativeUsdChange => ScenarioCategory::NativeSteadyState,

            Self::S11WebRtcIdleConnected
            | Self::S12WebRtcIdleClientConnected
            | Self::S13WebRtcRemoteGridVisibilityCommand
            | Self::S14WebRtcRemoteGroundOriginCommand
            | Self::S15WebRtcRemoteOrbitPan
            | Self::S16WebRtcRemoteVisuallyEmpty
            | Self::S17WebRtcRemoteAuthoritativeUsdEdit
            | Self::S18WebRtcRemoteCommandAfterLongIdle => ScenarioCategory::WebRtcRemotePath,

            Self::S19IsolationQuerySaturation
            | Self::S20IsolationAuthValidationBurst
            | Self::S21IsolationNavigationUnderAuth
            | Self::S22IsolationQueryCommandConcurrency
            | Self::S23IsolationSlowFailingDataWorker
            | Self::S24IsolationAuthRevocationPropagation => {
                ScenarioCategory::RenderDataPlaneIsolation
            }
        }
    }

    /// All 24 canonical scenarios in order.
    pub fn all() -> &'static [Self] {
        &[
            Self::S1NativeHummingbirdGridOnPaused,
            Self::S2NativeHummingbirdGridOffPaused,
            Self::S3NativeCameraOrbitPan,
            Self::S4NativeGridVisibilityToggle,
            Self::S5NativeGroundOriginChange,
            Self::S6NativeGridStyleColorChange,
            Self::S7NativeVisuallyEmptyLiveStageRetained,
            Self::S8NativeNoLiveStage,
            Self::S9NativeRecoveryIdle,
            Self::S10NativeAuthoritativeUsdChange,
            Self::S11WebRtcIdleConnected,
            Self::S12WebRtcIdleClientConnected,
            Self::S13WebRtcRemoteGridVisibilityCommand,
            Self::S14WebRtcRemoteGroundOriginCommand,
            Self::S15WebRtcRemoteOrbitPan,
            Self::S16WebRtcRemoteVisuallyEmpty,
            Self::S17WebRtcRemoteAuthoritativeUsdEdit,
            Self::S18WebRtcRemoteCommandAfterLongIdle,
            Self::S19IsolationQuerySaturation,
            Self::S20IsolationAuthValidationBurst,
            Self::S21IsolationNavigationUnderAuth,
            Self::S22IsolationQueryCommandConcurrency,
            Self::S23IsolationSlowFailingDataWorker,
            Self::S24IsolationAuthRevocationPropagation,
        ]
    }

    /// Looks up scenario by its code identifier (e.g. "S1", "S24").
    pub fn from_code(code: &str) -> Option<Self> {
        Self::all().iter().copied().find(|s| s.code() == code)
    }
}

/// Category classification for benchmark probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioCategory {
    NativeSteadyState,
    WebRtcRemotePath,
    RenderDataPlaneIsolation,
}

/// Invariant expectations for deterministic steady-state checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SteadyStateExpectations {
    pub grid_structural_rebuilds: u64,
    pub semantic_snapshot_clones: u64,
    pub recovery_checkpoints: u64,
    pub sync_db_auth_waits_in_bevy: u64,
}

/// Complete definition of a benchmark probe scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioProbeDefinition {
    pub id: BenchmarkScenarioId,
    pub code: String,
    pub title: String,
    pub category: ScenarioCategory,
    pub fixture_path: Option<String>,
    pub fixture_label: String,
    pub grid_enabled: bool,
    pub expected_steady_state: SteadyStateExpectations,
}

impl ScenarioProbeDefinition {
    fn new(id: BenchmarkScenarioId, title: &str, fixture: Option<&str>, grid: bool) -> Self {
        Self {
            id,
            code: id.code().to_string(),
            title: title.to_string(),
            category: id.category(),
            fixture_path: fixture.map(str::to_string),
            fixture_label: fixture
                .and_then(|p| p.split('/').next_back())
                .unwrap_or("no_stage")
                .to_string(),
            grid_enabled: grid,
            expected_steady_state: SteadyStateExpectations::default(),
        }
    }

    pub fn for_scenario(id: BenchmarkScenarioId) -> Self {
        scenario_definitions::for_scenario(id)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/diagnostics/performance/scenario_inline_tests.rs"]
mod tests;
