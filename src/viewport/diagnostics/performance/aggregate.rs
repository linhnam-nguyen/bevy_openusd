//! Aggregation functions and structured benchmark report schema.

use serde::{Deserialize, Serialize};

use super::sample::{BenchmarkIdentity, FrameSample, RenderConfiguration, SCHEMA_VERSION};
use super::scenario::SteadyStateExpectations;

/// Summary metrics for Incident A (GroundGrid churn).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IncidentGridSummary {
    pub compute_extent_calls: u64,
    pub sync_calls: u64,
    pub host_writes: u64,
    pub structural_rebuilds: u64,
    pub vertices_generated: u64,
    pub indices_generated: u64,
}

/// Summary metrics for Incident B (Semantic stage snapshot cloning).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IncidentSemanticSummary {
    pub sync_calls: u64,
    pub idle_skips: u64,
    pub snapshot_clones: u64,
    pub initial_extractions: u64,
    pub worker_submissions: u64,
    pub recovery_checkpoints: u64,
}

/// Summary metrics for WebRTC remote streaming path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebRtcReportSummary {
    pub remote_commands_drained: u64,
    pub remote_inputs_applied: u64,
    pub authoritative_events_published: u64,
    pub captured_frames: u64,
    pub frame_queue_drops: u64,
}

/// Summary metrics for Render / Data-Plane Isolation invariants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IsolationReportSummary {
    pub sync_db_auth_waits_in_bevy: u64,
    pub query_saturations: u64,
    pub auth_validation_bursts: u64,
    pub auth_lookup_count: u64,
}

/// Aggregated statistical timing metrics across measured frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameTimingAggregate {
    pub warmup_frames: u64,
    pub measured_frames: u64,
    pub median_frame_ms: f64,
    pub p95_frame_ms: f64,
    pub min_frame_ms: f64,
    pub max_frame_ms: f64,
    pub actual_renderer_fps: f64,
    pub avg_fps: f64,
    pub p95_fps_equivalent: f64,
    pub wall_median_ms: f64,
    pub wall_p95_ms: f64,
    pub gpu_median_frame_ms: Option<f64>,
    pub gpu_p95_frame_ms: Option<f64>,
}

/// Breakdown of stage projection and mesh generation phases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PhaseMetrics {
    pub initial_projection_ms: Option<f64>,
    pub initial_projection_prims: u64,
    pub stage_traversal_ms: Option<f64>,
    pub mesh_generation_ms: Option<f64>,
    pub primvar_expansion_ms: Option<f64>,
    pub normal_generation_ms: Option<f64>,
    pub material_resolve_ms: Option<f64>,
}

/// Snapshot of asset and geometry caches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CacheSnapshot {
    pub live_stage_prims: u64,
    pub live_stage_animated_prims: u64,
    pub cached_materials: u64,
    pub cached_textures: u64,
    pub material_hits: u64,
    pub material_misses: u64,
    pub texture_hits: u64,
    pub texture_misses: u64,
}

/// Top-level standardized performance report JSON root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub schema_version: u32,
    pub identity: BenchmarkIdentity,
    pub requested_configuration: RenderConfiguration,
    pub effective_configuration: RenderConfiguration,
    pub configuration_matches: bool,
    pub expected_steady_state: SteadyStateExpectations,
    pub observed_steady_state: SteadyStateExpectations,
    pub steady_state_matches: bool,
    pub timing: FrameTimingAggregate,
    pub incident_grid: IncidentGridSummary,
    pub incident_semantic: IncidentSemanticSummary,
    pub webrtc_metrics: WebRtcReportSummary,
    pub isolation_metrics: IsolationReportSummary,
    pub phase_metrics: PhaseMetrics,
    pub cache_snapshot: CacheSnapshot,
    pub raw_samples: Vec<FrameSample>,
}

/// Computes percentile value from a pre-sorted slice.
pub fn calculate_percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = pct * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

/// Calculates timing aggregate from raw frame samples and warmup count.
pub fn aggregate_frames(
    samples: &[FrameSample],
    warmup_count: usize,
) -> FrameTimingAggregate {
    let measured_samples = if samples.len() > warmup_count {
        &samples[warmup_count..]
    } else {
        samples
    };

    if measured_samples.is_empty() {
        return FrameTimingAggregate {
            warmup_frames: warmup_count as u64,
            measured_frames: 0,
            median_frame_ms: 0.0,
            p95_frame_ms: 0.0,
            min_frame_ms: 0.0,
            max_frame_ms: 0.0,
            actual_renderer_fps: 0.0,
            avg_fps: 0.0,
            p95_fps_equivalent: 0.0,
            wall_median_ms: 0.0,
            wall_p95_ms: 0.0,
            gpu_median_frame_ms: None,
            gpu_p95_frame_ms: None,
        };
    }

    let mut cpu_times: Vec<f64> = measured_samples.iter().map(|s| s.cpu_duration_ms).collect();
    cpu_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut wall_intervals: Vec<f64> = measured_samples
        .iter()
        .filter_map(|s| s.wall_interval_ms)
        .collect();
    wall_intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let min_frame_ms = *cpu_times.first().unwrap_or(&0.0);
    let max_frame_ms = *cpu_times.last().unwrap_or(&0.0);
    let median_frame_ms = calculate_percentile(&cpu_times, 0.50);
    let p95_frame_ms = calculate_percentile(&cpu_times, 0.95);

    let wall_median_ms = calculate_percentile(&wall_intervals, 0.50);
    let wall_p95_ms = calculate_percentile(&wall_intervals, 0.95);

    let actual_renderer_fps = if wall_median_ms > 0.0 {
        1000.0 / wall_median_ms
    } else if median_frame_ms > 0.0 {
        1000.0 / median_frame_ms
    } else {
        0.0
    };

    let avg_fps = if median_frame_ms > 0.0 {
        1000.0 / median_frame_ms
    } else {
        0.0
    };
    let p95_fps_equivalent = if p95_frame_ms > 0.0 {
        1000.0 / p95_frame_ms
    } else {
        0.0
    };

    FrameTimingAggregate {
        warmup_frames: warmup_count as u64,
        measured_frames: measured_samples.len() as u64,
        median_frame_ms,
        p95_frame_ms,
        min_frame_ms,
        max_frame_ms,
        actual_renderer_fps,
        avg_fps,
        p95_fps_equivalent,
        wall_median_ms,
        wall_p95_ms,
        gpu_median_frame_ms: None,
        gpu_p95_frame_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregation_median_and_p95_calculation() {
        let samples: Vec<FrameSample> = (1..=100)
            .map(|i| FrameSample {
                frame_index: i,
                cpu_duration_ms: i as f64,
                wall_interval_ms: Some(i as f64),
                gpu_duration_ms: None,
            })
            .collect();

        let agg = aggregate_frames(&samples, 10);
        assert_eq!(agg.warmup_frames, 10);
        assert_eq!(agg.measured_frames, 90);
        assert!((agg.median_frame_ms - 55.5).abs() < 0.1);
        assert!((agg.p95_frame_ms - 95.55).abs() < 0.1);
        assert_eq!(agg.min_frame_ms, 11.0);
        assert_eq!(agg.max_frame_ms, 100.0);
        assert!(agg.actual_renderer_fps > 0.0);
    }

    #[test]
    fn empty_sample_behavior_returns_zeroes_without_panic() {
        let agg = aggregate_frames(&[], 0);
        assert_eq!(agg.measured_frames, 0);
        assert_eq!(agg.median_frame_ms, 0.0);
        assert_eq!(agg.avg_fps, 0.0);
    }
}
