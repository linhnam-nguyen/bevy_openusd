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
    pub grid_sync_calls: u64,
    pub grid_host_writes: u64,
    pub grid_structural_rebuilds: u64,
    pub grid_vertices_generated: u64,
    pub grid_indices_generated: u64,

    // Incident B counters (Semantic sync & snapshot cloning)
    pub semantic_sync_calls: u64,
    pub semantic_idle_skips: u64,
    pub semantic_snapshot_clones: u64,
    pub semantic_initial_extractions: u64,
    pub semantic_worker_submissions: u64,
    pub recovery_checkpoints: u64,

    // WebRTC remote stream counters
    pub remote_commands_drained: u64,
    pub remote_inputs_applied: u64,
    pub authoritative_events_published: u64,
    pub captured_frames: u64,
    pub frame_queue_drops: u64,

    // Data Plane Isolation counters
    pub sync_db_auth_waits_in_bevy: u64,
    pub query_saturations: u64,
    pub auth_validation_bursts: u64,
    pub auth_lookup_count: u64,
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
            grid_sync_calls: 0,
            grid_host_writes: 0,
            grid_structural_rebuilds: 0,
            grid_vertices_generated: 0,
            grid_indices_generated: 0,

            semantic_sync_calls: 0,
            semantic_idle_skips: 0,
            semantic_snapshot_clones: 0,
            semantic_initial_extractions: 0,
            semantic_worker_submissions: 0,
            recovery_checkpoints: 0,

            remote_commands_drained: 0,
            remote_inputs_applied: 0,
            authoritative_events_published: 0,
            captured_frames: 0,
            frame_queue_drops: 0,

            sync_db_auth_waits_in_bevy: 0,
            query_saturations: 0,
            auth_validation_bursts: 0,
            auth_lookup_count: 0,
        }
    }
}

impl RendererCounters {
    /// Resets runtime metrics for measurement windows while preserving configuration.
    pub fn reset(&mut self) {
        self.frame_count = 0;
        self.measured_frame_count = 0;
        self.frame_start_instant = None;
        self.last_frame_instant = None;
        self.frame_cpu_duration_ms = 0.0;
        self.frame_wall_interval_ms = None;

        self.grid_compute_extent_calls = 0;
        self.grid_sync_calls = 0;
        self.grid_host_writes = 0;
        self.grid_structural_rebuilds = 0;
        self.grid_vertices_generated = 0;
        self.grid_indices_generated = 0;

        self.semantic_sync_calls = 0;
        self.semantic_idle_skips = 0;
        self.semantic_snapshot_clones = 0;
        self.semantic_initial_extractions = 0;
        self.semantic_worker_submissions = 0;
        self.recovery_checkpoints = 0;

        self.remote_commands_drained = 0;
        self.remote_inputs_applied = 0;
        self.authoritative_events_published = 0;
        self.captured_frames = 0;
        self.frame_queue_drops = 0;

        self.sync_db_auth_waits_in_bevy = 0;
        self.query_saturations = 0;
        self.auth_validation_bursts = 0;
        self.auth_lookup_count = 0;
    }
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
}
