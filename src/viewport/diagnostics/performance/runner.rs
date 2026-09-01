//! Automated benchmark stepping system and execution lifecycle runner.

use std::path::PathBuf;

use super::counters::RendererCounters;
use super::sample::FrameSample;
use super::scenario::BenchmarkScenarioId;
use crate::viewport::transport::FrameTransportResource;
use bevy::prelude::*;

#[path = "runner_finalize.rs"]
mod finalize;

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
                        && measured_frames >= config.target_frames.saturating_div(2).clamp(1, 60)
                })
            {
                run_state.measurement_idle_signaled = true;
                if let Some(path) = config
                    .as_ref()
                    .and_then(|config| config.measurement_idle_file.as_ref())
                {
                    finalize::touch_marker(path);
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
        if let Some(mut c) = world.get_resource_mut::<usd_bevy::PerformanceCounters>() {
            c.reset();
        }
        if let Some(mut gc) = world.get_resource_mut::<bevy_glacial::prelude::GlacialGridCounters>()
        {
            *gc = Default::default();
        }
        if let Some(frame_metrics) = world.get_resource::<FrameTransportResource>() {
            frame_metrics.0.reset();
        }
        if let Some(path) = config
            .as_ref()
            .and_then(|config| config.measurement_start_file.as_ref())
        {
            finalize::touch_marker(path);
        }
    }

    if should_finalize {
        finalize::finalize_benchmark_report(world);
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
