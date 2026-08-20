//! Renderer frame execution counters and incident observability resources.

use bevy::prelude::*;
use std::time::Instant;

/// Runtime counters for renderer frame pacing, incident diagnostics, WebRTC, and ECS health.
#[derive(Resource, Debug, Clone)]
pub struct RendererCounters {
    pub frame_count: u64,
    pub measured_frame_count: u64,

    pub frame_start_instant: Option<Instant>,
    pub last_frame_instant: Option<Instant>,
    pub frame_cpu_duration_ms: f64,
    pub frame_wall_interval_ms: Option<f64>,

    // Configuration flags observed
    pub configuration_grid_enabled: bool,
    pub configuration_shadows_enabled: bool,
    pub configuration_edges_enabled: bool,
    pub configuration_material_overrides: bool,

    // Incident A counters (GroundGrid)
    pub grid_compute_extent_calls: u64,
    pub grid_prims_scanned: u64,
    pub grid_sync_calls: u64,
    pub grid_host_writes: u64,
    pub grid_visible_writes: u64,
    pub grid_ground_y_writes: u64,
    pub grid_coverage_radius_writes: u64,
    pub grid_value_changes: u64,
    pub grid_changed_observations: u64,
    pub grid_update_alpha_calls: u64,
    pub grid_lines_rebuilt: u64,
    pub grid_dots_rebuilt: u64,
    pub grid_structural_rebuilds: u64,
    pub grid_vertices_generated: u64,
    pub grid_indices_generated: u64,

    // Incident B counters (Semantic sync & snapshot cloning)
    pub semantic_sync_calls: u64,
    pub semantic_idle_skips: u64,
    pub semantic_snapshot_clones: u64,
    pub semantic_initial_extractions: u64,
    pub semantic_initial_extraction_failures: u64,
    pub semantic_fallback_extractions: u64,
    pub semantic_subtree_extractions: u64,
    pub semantic_worker_submissions: u64,
    pub semantic_worker_submission_failures: u64,
    pub recovery_checkpoints: u64,
    pub recovery_successes: u64,

    // WebRTC remote stream counters
    pub remote_commands_drained: u64,
    pub remote_inputs_applied: u64,
    pub authoritative_events_published: u64,
    pub captured_frames: u64,
    pub frame_queue_drops: u64,

    // Data Plane Isolation counters
    pub sync_db_auth_waits_in_bevy: u64,
    pub query_saturations: u64,
    pub query_requests: u64,
    pub query_results: u64,
    pub query_failures: u64,
    pub query_high_water: u64,
    pub query_median_latency_ms: Option<f64>,
    pub query_p95_latency_ms: Option<f64>,
    query_latency_samples_ms: Vec<f64>,
    pub auth_validation_bursts: u64,
    pub auth_lookup_count: u64,
    pub auth_snapshot_hits: u64,
    pub auth_validations: u64,
    pub auth_failures: u64,
    pub auth_high_water: u64,
}

impl Default for RendererCounters {
    fn default() -> Self {
        Self {
            frame_count: 0,
            measured_frame_count: 0,

            frame_start_instant: None,
            last_frame_instant: None,
            frame_cpu_duration_ms: 0.0,
            frame_wall_interval_ms: None,

            configuration_grid_enabled: true,
            configuration_shadows_enabled: true,
            configuration_edges_enabled: false,
            configuration_material_overrides: true,

            grid_compute_extent_calls: 0,
            grid_prims_scanned: 0,
            grid_sync_calls: 0,
            grid_host_writes: 0,
            grid_visible_writes: 0,
            grid_ground_y_writes: 0,
            grid_coverage_radius_writes: 0,
            grid_value_changes: 0,
            grid_changed_observations: 0,
            grid_update_alpha_calls: 0,
            grid_lines_rebuilt: 0,
            grid_dots_rebuilt: 0,
            grid_structural_rebuilds: 0,
            grid_vertices_generated: 0,
            grid_indices_generated: 0,

            semantic_sync_calls: 0,
            semantic_idle_skips: 0,
            semantic_snapshot_clones: 0,
            semantic_initial_extractions: 0,
            semantic_initial_extraction_failures: 0,
            semantic_fallback_extractions: 0,
            semantic_subtree_extractions: 0,
            semantic_worker_submissions: 0,
            semantic_worker_submission_failures: 0,
            recovery_checkpoints: 0,
            recovery_successes: 0,

            remote_commands_drained: 0,
            remote_inputs_applied: 0,
            authoritative_events_published: 0,
            captured_frames: 0,
            frame_queue_drops: 0,

            sync_db_auth_waits_in_bevy: 0,
            query_saturations: 0,
            query_requests: 0,
            query_results: 0,
            query_failures: 0,
            query_high_water: 0,
            query_median_latency_ms: None,
            query_p95_latency_ms: None,
            query_latency_samples_ms: Vec::new(),
            auth_validation_bursts: 0,
            auth_lookup_count: 0,
            auth_snapshot_hits: 0,
            auth_validations: 0,
            auth_failures: 0,
            auth_high_water: 0,
        }
    }
}

impl RendererCounters {
    /// Resets runtime metrics for measurement windows while preserving configuration.
    pub fn reset(&mut self) {
        self.grid_compute_extent_calls = 0;
        self.grid_prims_scanned = 0;
        self.grid_sync_calls = 0;
        self.grid_host_writes = 0;
        self.grid_visible_writes = 0;
        self.grid_ground_y_writes = 0;
        self.grid_coverage_radius_writes = 0;
        self.grid_value_changes = 0;
        self.grid_changed_observations = 0;
        self.grid_update_alpha_calls = 0;
        self.grid_lines_rebuilt = 0;
        self.grid_dots_rebuilt = 0;
        self.grid_structural_rebuilds = 0;
        self.grid_vertices_generated = 0;
        self.grid_indices_generated = 0;

        self.semantic_sync_calls = 0;
        self.semantic_idle_skips = 0;
        self.semantic_snapshot_clones = 0;
        self.semantic_initial_extractions = 0;
        self.semantic_initial_extraction_failures = 0;
        self.semantic_fallback_extractions = 0;
        self.semantic_subtree_extractions = 0;
        self.semantic_worker_submissions = 0;
        self.semantic_worker_submission_failures = 0;
        self.recovery_checkpoints = 0;
        self.recovery_successes = 0;

        self.remote_commands_drained = 0;
        self.remote_inputs_applied = 0;
        self.authoritative_events_published = 0;
        self.captured_frames = 0;
        self.frame_queue_drops = 0;

        self.sync_db_auth_waits_in_bevy = 0;
        self.query_saturations = 0;
        self.query_requests = 0;
        self.query_results = 0;
        self.query_failures = 0;
        self.query_high_water = 0;
        self.query_median_latency_ms = None;
        self.query_p95_latency_ms = None;
        self.query_latency_samples_ms.clear();
        self.auth_validation_bursts = 0;
        self.auth_lookup_count = 0;
        self.auth_snapshot_hits = 0;
        self.auth_validations = 0;
        self.auth_failures = 0;
        self.auth_high_water = 0;
    }

    pub fn record_query_latency_ms(&mut self, latency_ms: f64) {
        if latency_ms.is_finite() && latency_ms >= 0.0 {
            self.query_latency_samples_ms.push(latency_ms);
        }
    }

    pub fn finalize_query_latency(&mut self) {
        if self.query_latency_samples_ms.is_empty() {
            self.query_median_latency_ms = None;
            self.query_p95_latency_ms = None;
            return;
        }

        self.query_latency_samples_ms
            .sort_by(|left, right| left.total_cmp(right));
        self.query_median_latency_ms = Some(percentile(
            &self.query_latency_samples_ms,
            0.50,
        ));
        self.query_p95_latency_ms = Some(percentile(
            &self.query_latency_samples_ms,
            0.95,
        ));
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = pct * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

/// Marks frame start instant in the `First` schedule.
pub fn start_frame_timing_system(mut counters: ResMut<RendererCounters>) {
    counters.frame_start_instant = Some(Instant::now());
}

/// Collects frame CPU duration and inter-frame wall-clock delta in the `Last` schedule.
pub fn collect_renderer_counters_system(mut counters: ResMut<RendererCounters>) {
    let now = Instant::now();
    counters.frame_count += 1;

    let cpu_duration = if let Some(start) = counters.frame_start_instant {
        now.duration_since(start).as_secs_f64() * 1000.0
    } else {
        0.0
    };
    counters.frame_cpu_duration_ms = cpu_duration;

    if let Some(last) = counters.last_frame_instant {
        let wall_interval = now.duration_since(last).as_secs_f64() * 1000.0;
        counters.frame_wall_interval_ms = Some(wall_interval);
    }
    counters.last_frame_instant = Some(now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_preserves_configuration_flags() {
        let mut counters = RendererCounters::default();
        counters.configuration_grid_enabled = false;
        counters.grid_structural_rebuilds = 42;
        counters.semantic_snapshot_clones = 15;

        counters.reset();

        assert_eq!(counters.grid_structural_rebuilds, 0);
        assert_eq!(counters.semantic_snapshot_clones, 0);
        assert!(!counters.configuration_grid_enabled);
    }

    #[test]
    fn query_latency_percentiles_are_derived_from_completed_samples() {
        let mut counters = RendererCounters::default();
        counters.record_query_latency_ms(9.0);
        counters.record_query_latency_ms(1.0);
        counters.record_query_latency_ms(5.0);

        counters.finalize_query_latency();

        assert_eq!(counters.query_median_latency_ms, Some(5.0));
        assert_eq!(counters.query_p95_latency_ms, Some(8.6));
    }
}
