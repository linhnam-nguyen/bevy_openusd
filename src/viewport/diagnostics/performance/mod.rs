//! Rendering performance diagnostics, steady-state invariant tracking, and benchmark suite.

pub mod aggregate;
pub mod collector;
pub mod counters;
pub mod decision;
pub mod runner;
pub mod sample;
pub mod scenario;
pub mod scenario_driver;

pub use aggregate::{
    CacheSnapshot, FrameTimingAggregate, IncidentGridSummary, IncidentSemanticSummary,
    IsolationReportSummary, PerformanceReport, PhaseMetrics, WebRtcReportSummary, aggregate_frames,
    calculate_percentile,
};
pub use collector::{
    ProjectionPhaseTimings, collect_cache_snapshot_from_world, collect_phase_metrics_from_world,
};
pub use counters::{
    RendererCounters, collect_renderer_counters_system, start_frame_timing_system,
};
pub use decision::{GroundGridDecisionHelper, SemanticDecisionHelper, SemanticSyncWorkAction};
pub use runner::{BenchmarkLaunchConfig, BenchmarkRunState, BenchmarkRunnerPlugin};
pub use sample::{BenchmarkIdentity, FrameSample, RenderConfiguration, RenderMode, SCHEMA_VERSION};
pub use scenario::{
    BenchmarkScenarioId, ScenarioCategory, ScenarioProbeDefinition, SteadyStateExpectations,
};
pub use scenario_driver::{
    ActiveScenarioDriver, ScenarioDriverPlugin, scenario_action_driver_system,
    setup_scenario_driver_system,
};
