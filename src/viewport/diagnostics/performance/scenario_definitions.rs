use super::{BenchmarkScenarioId, ScenarioProbeDefinition};

const HUMMINGBIRD: &str = "assets/external/hummingbird.usdz";
const EMPTY: &str = "tests/stages/empty.usda";

pub(super) fn for_scenario(id: BenchmarkScenarioId) -> ScenarioProbeDefinition {
    let (title, fixture, grid) = match id {
        BenchmarkScenarioId::S1NativeHummingbirdGridOnPaused => (
            "Hummingbird steady-state with grid ON (paused)",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S2NativeHummingbirdGridOffPaused => (
            "Hummingbird steady-state with grid OFF (paused)",
            Some(HUMMINGBIRD),
            false,
        ),
        BenchmarkScenarioId::S3NativeCameraOrbitPan => (
            "Native Camera Orbit and Pan navigation active",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S4NativeGridVisibilityToggle => (
            "Native Grid Visibility toggling every 15 frames",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S5NativeGroundOriginChange => (
            "Native Ground Origin mutating every 10 frames",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S6NativeGridStyleColorChange => (
            "Native Grid Style Color mutating every 10 frames",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S7NativeVisuallyEmptyLiveStageRetained => (
            "Visually empty stage with LiveStage retained",
            Some(EMPTY),
            true,
        ),
        BenchmarkScenarioId::S8NativeNoLiveStage => {
            ("Viewer startup without active LiveStage", None, true)
        }
        BenchmarkScenarioId::S9NativeRecoveryIdle => (
            "Native Recovery Idle without authoring edits",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S10NativeAuthoritativeUsdChange => (
            "Native Authoritative USD Change applied",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S11WebRtcIdleConnected => (
            "WebRTC Headless Server idle connected",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S12WebRtcIdleClientConnected => (
            "WebRTC Remote client connected stream idle",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S13WebRtcRemoteGridVisibilityCommand => (
            "WebRTC Remote Grid Visibility command stream",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S14WebRtcRemoteGroundOriginCommand => (
            "WebRTC Remote Ground Origin command stream",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S15WebRtcRemoteOrbitPan => (
            "WebRTC Remote Orbit/Pan client input stream",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S16WebRtcRemoteVisuallyEmpty => (
            "WebRTC Remote visually empty stage retained",
            Some(EMPTY),
            true,
        ),
        BenchmarkScenarioId::S17WebRtcRemoteAuthoritativeUsdEdit => (
            "WebRTC Remote authoritative stage mutation edit",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S18WebRtcRemoteCommandAfterLongIdle => (
            "WebRTC Remote command after long idle duration",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S19IsolationQuerySaturation => (
            "Isolation: High-throughput semantic queries during render",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S20IsolationAuthValidationBurst => (
            "Isolation: Authentication validation burst load",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S21IsolationNavigationUnderAuth => (
            "Isolation: Viewport navigation under auth check pressure",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S22IsolationQueryCommandConcurrency => (
            "Isolation: Concurrent query and editor command batches",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S23IsolationSlowFailingDataWorker => (
            "Isolation: Slow or failing background semantic worker",
            Some(HUMMINGBIRD),
            true,
        ),
        BenchmarkScenarioId::S24IsolationAuthRevocationPropagation => (
            "Isolation: Auth token revocation propagation",
            Some(HUMMINGBIRD),
            true,
        ),
    };
    ScenarioProbeDefinition::new(id, title, fixture, grid)
}
