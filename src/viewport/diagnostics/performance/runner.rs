//! Automated benchmark stepping system and execution lifecycle runner.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;

use super::aggregate::{
    IncidentGridSummary, IncidentSemanticSummary, IsolationReportSummary, PerformanceReport,
    RendererCadenceSummary, WebRtcReportSummary, aggregate_frames,
};
use super::collector::{collect_cache_snapshot_from_world, collect_phase_metrics_from_world};
use super::counters::RendererCounters;
use super::sample::{BenchmarkIdentity, FrameSample, RenderConfiguration, RenderMode};
use super::scenario::{BenchmarkScenarioId, ScenarioProbeDefinition, SteadyStateExpectations};
use crate::viewport::semantic::SemanticWorkingStore;

/// Launch options configuring automated benchmark execution.
#[derive(Resource, Debug, Clone)]
pub struct BenchmarkLaunchConfig {
    pub scenario: Option<BenchmarkScenarioId>,
    pub renderer_matrix: bool,
    pub mesh_profile: bool,
    pub warmup_frames: u64,
    pub target_frames: u64,
    pub output_path: Option<PathBuf>,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub requested_fps: f64,
    pub asset_path: Option<String>,
    pub client_ready_file: Option<PathBuf>,
    pub measurement_start_file: Option<PathBuf>,
    pub measurement_idle_file: Option<PathBuf>,
    pub measurement_complete_file: Option<PathBuf>,
}

/// Dynamic runtime state of the benchmark execution.
#[derive(Resource, Debug, Clone)]
pub struct BenchmarkRunState {
    pub scene_ready: bool,
    pub client_ready: bool,
    pub measurement_started: bool,
    pub measurement_idle_signaled: bool,
    pub warmup_frames_remaining: u64,
    pub target_frames_remaining: u64,
    pub samples: Vec<FrameSample>,
    pub is_completed: bool,
}

impl BenchmarkRunState {
    pub fn new(warmup: u64, target: u64) -> Self {
        Self {
            scene_ready: false,
            client_ready: false,
            measurement_started: false,
            measurement_idle_signaled: false,
            warmup_frames_remaining: warmup,
            target_frames_remaining: target,
            samples: Vec::with_capacity(target as usize + warmup as usize),
            is_completed: false,
        }
    }
}

/// Exclusive Bevy system executing at `Last` to capture frames and exit when complete.
pub fn benchmark_stepper_system(world: &mut World) {
    let config = world.get_resource::<BenchmarkLaunchConfig>().cloned();
    let is_s8 = config
        .as_ref()
        .and_then(|c| c.scenario)
        .map(|s| s == BenchmarkScenarioId::S8NativeNoLiveStage)
        .unwrap_or(false);
    let scene_count = world
        .get_resource::<crate::viewport::scene::SceneExtent>()
        .map(|e| e.count)
        .unwrap_or(0);
    let has_live = world.get_non_send::<usd_bevy::LiveStage>().is_some();
    let is_ready = is_s8 || scene_count > 0 || has_live;

    let mut should_finalize = false;
    let mut warmup_finished = false;

    if let (Some(counters), Some(mut run_state)) = (
        world.get_resource::<RendererCounters>().cloned(),
        world.get_resource_mut::<BenchmarkRunState>(),
    ) {
        if config
            .as_ref()
            .is_some_and(|config| config.client_ready_file.is_some())
            && !run_state.client_ready
        {
            let Some(path) = config
                .as_ref()
                .and_then(|config| config.client_ready_file.as_ref())
            else {
                return;
            };
            if !path.exists() {
                return;
            }
            run_state.client_ready = true;
        }

        if !run_state.scene_ready {
            if is_ready {
                run_state.scene_ready = true;
            } else {
                return;
            }
        }

        if run_state.warmup_frames_remaining > 0 {
            run_state.warmup_frames_remaining -= 1;
            run_state.samples.push(FrameSample {
                frame_index: counters.frame_count,
                cpu_duration_ms: counters.frame_cpu_duration_ms,
                wall_interval_ms: counters.frame_wall_interval_ms,
                gpu_duration_ms: None,
            });
            if run_state.warmup_frames_remaining == 0 {
                run_state.measurement_started = true;
                warmup_finished = true;
            }
        } else if run_state.target_frames_remaining > 0 {
            run_state.target_frames_remaining -= 1;
            let measured_frames = config.as_ref().map_or(0, |config| {
                config
                    .target_frames
                    .saturating_sub(run_state.target_frames_remaining)
            });
            if !run_state.measurement_idle_signaled
                && config.as_ref().is_some_and(|config| {
                    config.measurement_idle_file.is_some()
                        && measured_frames >= config.target_frames.saturating_div(2).max(1).min(60)
                })
            {
                run_state.measurement_idle_signaled = true;
                if let Some(path) = config
                    .as_ref()
                    .and_then(|config| config.measurement_idle_file.as_ref())
                {
                    touch_marker(path);
                }
            }
            run_state.samples.push(FrameSample {
                frame_index: counters.frame_count,
                cpu_duration_ms: counters.frame_cpu_duration_ms,
                wall_interval_ms: counters.frame_wall_interval_ms,
                gpu_duration_ms: None,
            });
            if run_state.target_frames_remaining == 0 {
                run_state.is_completed = true;
                should_finalize = true;
            }
        }
    }

    if warmup_finished {
        // Reset counters at the warm-up boundary so reported steady-state
        // metrics represent only post-warm-up measured frames.
        if let Some(mut c) = world.get_resource_mut::<RendererCounters>() {
            c.reset();
        }
        if let Some(mut gc) = world.get_resource_mut::<bevy_glacial::prelude::GlacialGridCounters>()
        {
            *gc = Default::default();
        }
        if let Some(path) = config
            .as_ref()
            .and_then(|config| config.measurement_start_file.as_ref())
        {
            touch_marker(path);
        }
    }

    if should_finalize {
        finalize_benchmark_report(world);
    }
}

fn finalize_benchmark_report(world: &mut World) {
    let config = world
        .get_resource::<BenchmarkLaunchConfig>()
        .cloned()
        .expect("BenchmarkLaunchConfig must exist");
    let run_state = world
        .get_resource::<BenchmarkRunState>()
        .cloned()
        .expect("BenchmarkRunState must exist");
    let mut counters = world
        .get_resource::<RendererCounters>()
        .cloned()
        .expect("RendererCounters must exist");
    if let Some(semantic_store) = world.get_resource::<SemanticWorkingStore>() {
        counters.query_high_water = semantic_store.query_queue_high_water();
    }
    counters.finalize_query_latency();

    let grid_resource = world.get_resource::<GroundGrid>();
    let grid_visible = grid_resource.map(|g| g.visible).unwrap_or(false);

    let scenario_def = config.scenario.map(ScenarioProbeDefinition::for_scenario);
    let scenario_code = config.scenario.map(|s| s.code().to_string());
    let scene_label = config
        .asset_path
        .clone()
        .unwrap_or_else(|| "no_stage".to_string());

    let requested_config = RenderConfiguration {
        grid: scenario_def
            .as_ref()
            .map(|d| d.grid_enabled)
            .unwrap_or(true),
        shadows: true,
        edges: false,
        render_mode: RenderMode::Shaded,
        material_overrides: true,
    };

    let effective_config = RenderConfiguration {
        grid: grid_visible,
        shadows: counters.configuration_shadows_enabled,
        edges: counters.configuration_edges_enabled,
        render_mode: RenderMode::Shaded,
        material_overrides: counters.configuration_material_overrides,
    };

    let configuration_matches = requested_config == effective_config;

    let expected_steady_state = scenario_def
        .map(|d| d.expected_steady_state)
        .unwrap_or_default();

    let observed_steady_state = SteadyStateExpectations {
        grid_structural_rebuilds: counters.grid_structural_rebuilds,
        semantic_snapshot_clones: counters.semantic_snapshot_clones,
        recovery_checkpoints: counters.recovery_checkpoints,
        sync_db_auth_waits_in_bevy: counters.sync_db_auth_waits_in_bevy,
    };

    let steady_state_matches =
        expected_steady_state == observed_steady_state && configuration_matches;

    let timing = aggregate_frames(&run_state.samples, config.warmup_frames as usize);

    let renderer_cadence = RendererCadenceSummary {
        requested_fps: counters.requested_renderer_fps,
        effective_renderer_target_fps: counters.effective_renderer_target_fps,
        actual_rendered_fps: counters
            .actual_rendered_fps
            .or((timing.actual_renderer_fps > 0.0).then_some(timing.actual_renderer_fps)),
        encoded_fps: counters.encoded_fps,
    };

    let incident_grid = IncidentGridSummary {
        compute_extent_calls: counters.grid_compute_extent_calls,
        prims_scanned: counters.grid_prims_scanned,
        sync_calls: counters.grid_sync_calls,
        host_writes: counters.grid_host_writes,
        visible_writes: counters.grid_visible_writes,
        ground_y_writes: counters.grid_ground_y_writes,
        coverage_radius_writes: counters.grid_coverage_radius_writes,
        value_changes: counters.grid_value_changes,
        changed_observations: counters.grid_changed_observations,
        update_alpha_calls: counters.grid_update_alpha_calls,
        lines_rebuilt: counters.grid_lines_rebuilt,
        dots_rebuilt: counters.grid_dots_rebuilt,
        structural_rebuilds: counters.grid_structural_rebuilds,
        vertices_generated: counters.grid_vertices_generated,
        indices_generated: counters.grid_indices_generated,
    };

    let incident_semantic = IncidentSemanticSummary {
        sync_calls: counters.semantic_sync_calls,
        idle_skips: counters.semantic_idle_skips,
        snapshot_clones: counters.semantic_snapshot_clones,
        initial_extractions: counters.semantic_initial_extractions,
        initial_extraction_failures: counters.semantic_initial_extraction_failures,
        fallback_extractions: counters.semantic_fallback_extractions,
        subtree_extractions: counters.semantic_subtree_extractions,
        worker_submissions: counters.semantic_worker_submissions,
        worker_submission_failures: counters.semantic_worker_submission_failures,
        recovery_checkpoints: counters.recovery_checkpoints,
        recovery_successes: counters.recovery_successes,
    };

    let webrtc_metrics = WebRtcReportSummary {
        remote_commands_drained: counters.remote_commands_drained,
        remote_inputs_applied: counters.remote_inputs_applied,
        authoritative_events_published: counters.authoritative_events_published,
        first_remote_command_received_frame: counters.first_remote_command_received_frame,
        first_authoritative_event_published_frame: counters
            .first_authoritative_event_published_frame,
        captured_frames: counters.captured_frames,
        frame_queue_drops: counters.frame_queue_drops,
    };

    let isolation_metrics = IsolationReportSummary {
        sync_db_auth_waits_in_bevy: counters.sync_db_auth_waits_in_bevy,
        query_saturations: counters.query_saturations,
        query_requests: counters.query_requests,
        query_results: counters.query_results,
        query_failures: counters.query_failures,
        query_high_water: counters.query_high_water,
        query_median_latency_ms: counters.query_median_latency_ms,
        query_p95_latency_ms: counters.query_p95_latency_ms,
        auth_validation_bursts: counters.auth_validation_bursts,
        auth_lookup_count: counters.auth_lookup_count,
        auth_snapshot_hits: counters.auth_snapshot_hits,
        auth_validations: counters.auth_validations,
        auth_failures: counters.auth_failures,
        auth_high_water: counters.auth_high_water,
    };

    let phase_metrics = collect_phase_metrics_from_world(world);
    let geometry_profile = config
        .mesh_profile
        .then(|| {
            world
                .get_resource::<usd_bevy::GeometryProfile>()
                .filter(|profile| profile.enabled)
                .cloned()
        })
        .flatten();
    let geometry_render_preparation = super::render_profile::snapshot(world, config.mesh_profile);
    let cache_snapshot = collect_cache_snapshot_from_world(world);
    let (gpu_adapter, backend) = if let Some(adapter_info) =
        world.get_resource::<bevy::render::renderer::RenderAdapterInfo>()
    {
        (
            adapter_info.name.clone(),
            format!("{:?}", adapter_info.backend).to_lowercase(),
        )
    } else {
        ("unknown".to_string(), "unknown".to_string())
    };

    let mut identity = BenchmarkIdentity::new(
        &config.label,
        &scene_label,
        scenario_code,
        gpu_adapter,
        config.width,
        config.height,
        config.requested_fps,
    );
    identity.backend = backend;

    let report = PerformanceReport {
        schema_version: 1,
        identity,
        requested_configuration: requested_config,
        effective_configuration: effective_config,
        configuration_matches,
        expected_steady_state,
        observed_steady_state,
        steady_state_matches,
        timing,
        incident_grid,
        incident_semantic,
        webrtc_metrics,
        renderer_cadence,
        isolation_metrics,
        phase_metrics,
        geometry_profile,
        geometry_render_preparation,
        cache_snapshot,
        raw_samples: run_state.samples,
    };

    if let Some(ref path) = config.output_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = File::create(path) {
            let json = serde_json::to_string_pretty(&report).unwrap_or_default();
            let _ = file.write_all(json.as_bytes());
        }
    }

    if let Some(path) = config.measurement_complete_file.as_ref() {
        touch_marker(path);
    }

    std::process::exit(0);
}

fn touch_marker(path: &PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = File::create(path);
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
