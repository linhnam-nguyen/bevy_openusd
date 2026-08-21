//! Release benchmark for the M3 renderer configuration matrix and cadence.

use bevy::prelude::*;
use viewport_protocol::{RenderMode as ProtocolRenderMode, RendererConfiguration, ViewportCommand};

use super::aggregate::RendererCadenceSummary;
use super::matrix_probe::{effective_renderer_configuration, matrix_identity};
use super::matrix_report::{RendererMatrixCadenceReport, RendererMatrixCaseReport};
use super::runner::BenchmarkLaunchConfig;
use super::sample::{RenderConfiguration, RenderMode};
use crate::viewport::api::ViewportCommandInbox;
use crate::viewport::app::cadence::RendererCadence;
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::scene::visualization::DisplayToggles;

const MATRIX_FPS_SAMPLES: [u32; 3] = [30, 60, 120];

#[derive(Debug, Clone)]
enum MatrixPhase {
    QueueConfiguration(usize),
    AwaitConfiguration {
        index: usize,
        frames_waited: u64,
    },
    WarmupConfiguration {
        index: usize,
        remaining: u64,
    },
    MeasureConfiguration {
        index: usize,
        remaining: u64,
        measured_frames: u64,
    },
    QueueCadence(usize),
    AwaitCadence {
        index: usize,
        frames_waited: u64,
    },
    WarmupCadence {
        index: usize,
        remaining: u64,
    },
    MeasureCadence {
        index: usize,
        remaining: u64,
        measured_frames: u64,
        actual_fps_sum: f64,
        actual_fps_samples: u64,
    },
    Complete,
}

#[derive(Resource, Debug, Clone)]
pub struct RendererMatrixRun {
    phase: MatrixPhase,
    cases: Vec<RendererMatrixCaseReport>,
    cadence_samples: Vec<RendererMatrixCadenceReport>,
}

impl RendererMatrixRun {
    pub fn new() -> Self {
        Self {
            phase: MatrixPhase::QueueConfiguration(0),
            cases: Vec::with_capacity(16),
            cadence_samples: Vec::with_capacity(MATRIX_FPS_SAMPLES.len()),
        }
    }
}

pub fn renderer_matrix_stepper_system(world: &mut World) {
    let Some(config) = world.get_resource::<BenchmarkLaunchConfig>().cloned() else {
        return;
    };
    let Some(mut run) = world.remove_resource::<RendererMatrixRun>() else {
        return;
    };

    let ready = benchmark_ready(world, &config);
    if ready {
        advance_matrix(world, &config, &mut run);
    }

    let complete = matches!(run.phase, MatrixPhase::Complete);
    world.insert_resource(run);
    if complete {
        let run = world.resource::<RendererMatrixRun>();
        let identity = matrix_identity(world, &config);
        super::matrix_report::finalize_matrix_report(
            &config,
            identity,
            &run.cases,
            &run.cadence_samples,
        );
    }
}

fn benchmark_ready(world: &World, config: &BenchmarkLaunchConfig) -> bool {
    let has_stage = world
        .get_resource::<crate::viewport::scene::SceneExtent>()
        .is_some_and(|extent| extent.count > 0);
    let has_live_stage = world.get_non_send::<usd_bevy::LiveStage>().is_some();
    if !(has_stage || has_live_stage) {
        return false;
    }
    config
        .client_ready_file
        .as_ref()
        .is_none_or(|path| path.exists())
}

fn advance_matrix(world: &mut World, config: &BenchmarkLaunchConfig, run: &mut RendererMatrixRun) {
    let phase = run.phase.clone();
    match phase {
        MatrixPhase::QueueConfiguration(index) => {
            queue_configuration(world, matrix_configuration(index));
            run.phase = MatrixPhase::AwaitConfiguration {
                index,
                frames_waited: 0,
            };
        }
        MatrixPhase::AwaitConfiguration {
            index,
            frames_waited,
        } => {
            if effective_renderer_configuration(world) == matrix_configuration(index) {
                reset_counters(world);
                run.phase = MatrixPhase::WarmupConfiguration {
                    index,
                    remaining: config.warmup_frames,
                };
            } else if frames_waited >= config.target_frames.max(1) {
                run.cases.push(failed_case(index));
                run.phase = MatrixPhase::Complete;
            } else {
                run.phase = MatrixPhase::AwaitConfiguration {
                    index,
                    frames_waited: frames_waited.saturating_add(1),
                };
            }
        }
        MatrixPhase::WarmupConfiguration { index, remaining } => {
            if remaining == 0 {
                reset_counters(world);
                run.phase = MatrixPhase::MeasureConfiguration {
                    index,
                    remaining: config.target_frames,
                    measured_frames: 0,
                };
            } else {
                run.phase = MatrixPhase::WarmupConfiguration {
                    index,
                    remaining: remaining.saturating_sub(1),
                };
            }
        }
        MatrixPhase::MeasureConfiguration {
            index,
            remaining,
            measured_frames,
        } => {
            if remaining == 0 {
                let requested = matrix_configuration(index);
                let effective = effective_renderer_configuration(world);
                run.cases.push(RendererMatrixCaseReport {
                    accepted: effective == requested,
                    configuration_matches: effective == requested,
                    requested,
                    effective,
                    measured_frames,
                });
                run.phase = if index + 1 < 16 {
                    MatrixPhase::QueueConfiguration(index + 1)
                } else {
                    MatrixPhase::QueueCadence(0)
                };
            } else {
                run.phase = MatrixPhase::MeasureConfiguration {
                    index,
                    remaining: remaining.saturating_sub(1),
                    measured_frames: measured_frames.saturating_add(1),
                };
            }
        }
        MatrixPhase::QueueCadence(index) => {
            queue_fps_configuration(world, MATRIX_FPS_SAMPLES[index]);
            run.phase = MatrixPhase::AwaitCadence {
                index,
                frames_waited: 0,
            };
        }
        MatrixPhase::AwaitCadence {
            index,
            frames_waited,
        } => {
            let requested_fps = MATRIX_FPS_SAMPLES[index];
            let effective_fps = world
                .get_resource::<RendererCadence>()
                .and_then(RendererCadence::effective_renderer_target_fps);
            if effective_fps == Some(requested_fps) {
                reset_counters(world);
                run.phase = MatrixPhase::WarmupCadence {
                    index,
                    remaining: config.warmup_frames,
                };
            } else if frames_waited >= config.target_frames.max(1) {
                run.cadence_samples.push(failed_cadence(requested_fps));
                run.phase = MatrixPhase::Complete;
            } else {
                run.phase = MatrixPhase::AwaitCadence {
                    index,
                    frames_waited: frames_waited.saturating_add(1),
                };
            }
        }
        MatrixPhase::WarmupCadence { index, remaining } => {
            if remaining == 0 {
                reset_counters(world);
                run.phase = MatrixPhase::MeasureCadence {
                    index,
                    remaining: config.target_frames,
                    measured_frames: 0,
                    actual_fps_sum: 0.0,
                    actual_fps_samples: 0,
                };
            } else {
                run.phase = MatrixPhase::WarmupCadence {
                    index,
                    remaining: remaining.saturating_sub(1),
                };
            }
        }
        MatrixPhase::MeasureCadence {
            index,
            remaining,
            measured_frames,
            actual_fps_sum,
            actual_fps_samples,
        } => {
            let counters = world.resource::<RendererCounters>();
            let (next_sum, next_samples) = counters
                .actual_rendered_fps
                .map_or((actual_fps_sum, actual_fps_samples), |fps| {
                    (actual_fps_sum + fps, actual_fps_samples + 1)
                });
            if remaining == 0 {
                let requested_fps = MATRIX_FPS_SAMPLES[index];
                let cadence = world.resource::<RendererCadence>();
                let actual =
                    (next_samples > 0).then_some(next_sum / f64::from(next_samples as u32));
                run.cadence_samples.push(RendererMatrixCadenceReport {
                    requested_fps,
                    measured_frames,
                    summary: RendererCadenceSummary {
                        requested_fps: Some(requested_fps),
                        effective_renderer_target_fps: cadence.effective_renderer_target_fps(),
                        actual_rendered_fps: actual,
                        encoded_fps: counters.encoded_fps,
                    },
                });
                run.phase = if index + 1 < MATRIX_FPS_SAMPLES.len() {
                    MatrixPhase::QueueCadence(index + 1)
                } else {
                    MatrixPhase::Complete
                };
            } else {
                run.phase = MatrixPhase::MeasureCadence {
                    index,
                    remaining: remaining.saturating_sub(1),
                    measured_frames: measured_frames.saturating_add(1),
                    actual_fps_sum: next_sum,
                    actual_fps_samples: next_samples,
                };
            }
        }
        MatrixPhase::Complete => {}
    }
}

fn matrix_configuration(index: usize) -> RenderConfiguration {
    let grid = index & 1 != 0;
    let shadows = index & 2 != 0;
    let edges = index & 4 != 0;
    let render_mode = if index & 8 != 0 {
        RenderMode::Wireframe
    } else {
        RenderMode::Shaded
    };
    RenderConfiguration {
        grid,
        shadows,
        edges,
        render_mode,
        material_overrides: true,
    }
}

fn queue_configuration(world: &mut World, requested: RenderConfiguration) {
    let preferred_fps = world
        .get_resource::<RendererCadence>()
        .and_then(RendererCadence::effective_renderer_target_fps);
    world
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SetRendererConfiguration {
            configuration: RendererConfiguration {
                grid: requested.grid,
                shadows: requested.shadows,
                edges: requested.edges,
                render_mode: match requested.render_mode {
                    RenderMode::Shaded => ProtocolRenderMode::Shaded,
                    RenderMode::Wireframe | RenderMode::Flat => ProtocolRenderMode::Wireframe,
                },
                preferred_fps,
            },
        });
}

fn queue_fps_configuration(world: &mut World, fps: u32) {
    let mut configuration = world.resource::<DisplayToggles>().renderer;
    configuration.preferred_fps = Some(fps);
    world
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SetRendererConfiguration { configuration });
}

fn reset_counters(world: &mut World) {
    world.resource_mut::<RendererCounters>().reset();
}

fn failed_case(index: usize) -> RendererMatrixCaseReport {
    let requested = matrix_configuration(index);
    RendererMatrixCaseReport {
        requested,
        effective: RenderConfiguration::default(),
        accepted: false,
        configuration_matches: false,
        measured_frames: 0,
    }
}

fn failed_cadence(requested_fps: u32) -> RendererMatrixCadenceReport {
    RendererMatrixCadenceReport {
        requested_fps,
        summary: RendererCadenceSummary {
            requested_fps: Some(requested_fps),
            ..Default::default()
        },
        measured_frames: 0,
    }
}

#[cfg(test)]
#[path = "matrix_tests.rs"]
mod tests;
