//! Rendering performance diagnostics, steady-state invariant tracking, and benchmark suite.

#![allow(unused_imports)]

#[path = "metrics/aggregate.rs"]
pub mod aggregate;
#[path = "metrics/collector.rs"]
pub mod collector;
#[path = "metrics/counters.rs"]
pub mod counters;
pub mod decision;
pub mod matrix;
#[path = "matrix/probe.rs"]
mod matrix_probe;
#[path = "matrix/report.rs"]
mod matrix_report;
mod render_profile;
pub mod runner;
#[path = "runner/finalize.rs"]
mod runner_finalize;
#[path = "runner/plugin.rs"]
mod runner_plugin;
#[path = "metrics/sample.rs"]
pub mod sample;
pub mod scenario;
#[path = "scenario/definitions.rs"]
mod scenario_definitions;
#[path = "scenario/driver.rs"]
pub mod scenario_driver;
#[path = "scenario/setup.rs"]
mod scenario_driver_setup;

pub mod metrics;
pub use aggregate::{
    CacheSnapshot, FrameTimingAggregate, IncidentGridSummary, IncidentSemanticSummary,
    IsolationReportSummary, PerformanceReport, PhaseMetrics, RendererCadenceSummary,
    WebRtcReportSummary, aggregate_frames, calculate_percentile,
};
pub use collector::{
    ProjectionPhaseTimings, collect_cache_snapshot_from_world, collect_phase_metrics_from_world,
};
pub use counters::{RendererCounters, collect_renderer_counters_system, start_frame_timing_system};
pub use decision::{GroundGridDecisionHelper, SemanticDecisionHelper, SemanticSyncWorkAction};
pub use matrix::RendererMatrixRun;
pub use matrix_report::{
    RendererMatrixCadenceReport, RendererMatrixCaseReport, RendererMatrixReport,
};
pub use render_profile::GeometryRenderPreparation;
pub use runner::{BenchmarkLaunchConfig, BenchmarkRunState};
pub use runner_plugin::BenchmarkRunnerPlugin;
pub use sample::{BenchmarkIdentity, FrameSample, RenderConfiguration, RenderMode, SCHEMA_VERSION};
pub use scenario::{
    BenchmarkScenarioId, ScenarioCategory, ScenarioProbeDefinition, SteadyStateExpectations,
};
pub use scenario_driver::{
    ActiveScenarioDriver, ScenarioDriverPlugin, scenario_action_driver_system,
    setup_scenario_driver_system,
};
