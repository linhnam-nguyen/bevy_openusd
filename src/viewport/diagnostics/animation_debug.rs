//! Opt-in runtime witness for the C1-to-C4 animation evidence chain.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::LiveStage;
use viewport_protocol::AnimationDebugSnapshot;

#[path = "capture.rs"]
mod capture;
#[path = "report.rs"]
mod report;

use crate::viewport::{
    animation::UsdStageTime,
    transport::{
        FrameTransportResource,
        frame_signature::{FrameSampleId, FrameSignatureDiagnostic},
    },
};

/// Calibration constants are deliberately conservative: the motion floor is
/// below the smallest observed fixture delta, while repeat/static ceilings are
/// above the fixed-camera readback noise band. Recalibration must update these
/// constants and the report together.
pub(crate) const HUMMINGBIRD_MIN_MAD: f64 = 1.0;
pub(crate) const HUMMINGBIRD_MAX_REPEAT_MAD: f64 = 2.0;
pub(crate) const STATIC_MAX_MAD: f64 = 1.0;
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DebugPhase {
    WaitingForAnimatedStage,
    Capturing(FrameSampleId),
    WaitingForStaticStage,
    WaitingForRestartStage,
    WaitingForClient,
    Complete,
}

#[derive(Resource)]
pub(crate) struct AnimationDebugRuntime {
    pub(crate) client_snapshots: Vec<AnimationDebugSnapshot>,
    server_snapshots: Vec<AnimationDebugSnapshot>,
    output_path: Option<PathBuf>,
    started_at: Instant,
    phase: DebugPhase,
    initial_identity: Option<(u64, u64)>,
    static_identity: Option<(u64, u64)>,
    restart_identity: Option<(u64, u64)>,
    expected_identity: Option<(u64, u64)>,
    expected_projection_generation: Option<u64>,
    initial_times: Option<(f64, f64, f64)>,
    static_times: Option<(f64, f64)>,
    diagnostic_error: Option<String>,
}

impl AnimationDebugRuntime {
    fn new(output_path: Option<String>) -> Self {
        Self {
            client_snapshots: Vec::with_capacity(5),
            server_snapshots: Vec::with_capacity(5),
            output_path: output_path.map(PathBuf::from),
            started_at: Instant::now(),
            phase: DebugPhase::WaitingForAnimatedStage,
            initial_identity: None,
            static_identity: None,
            restart_identity: None,
            expected_identity: None,
            expected_projection_generation: None,
            initial_times: None,
            static_times: None,
            diagnostic_error: None,
        }
    }
}

pub(crate) struct AnimationDebugPlugin {
    pub(crate) output_path: Option<String>,
}

impl Plugin for AnimationDebugPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AnimationDebugRuntime::new(self.output_path.clone()))
            .add_systems(
                Update,
                drive_animation_debug
                    .after(usd_bevy::LiveStageSet::Animation)
                    .before(crate::viewport::api::ViewportBridgeSet::ReduceEvents),
            );
    }
}

fn drive_animation_debug(world: &mut World) {
    let timed_out = world
        .resource::<AnimationDebugRuntime>()
        .started_at
        .elapsed()
        > DIAGNOSTIC_TIMEOUT;
    if timed_out {
        capture::finish_report(world);
        return;
    }

    let phase = world.resource::<AnimationDebugRuntime>().phase;
    match phase {
        DebugPhase::WaitingForAnimatedStage => start_animated_samples(world),
        DebugPhase::WaitingForStaticStage => start_static_samples(world),
        DebugPhase::WaitingForRestartStage => {
            let expected = world.resource::<AnimationDebugRuntime>().expected_identity;
            let minimum_projection_generation = world
                .resource::<AnimationDebugRuntime>()
                .expected_projection_generation;
            if let Some((identity, ..)) =
                capture::ready_stage(world, true, expected, minimum_projection_generation)
            {
                let mut runtime = world.resource_mut::<AnimationDebugRuntime>();
                runtime.restart_identity = Some(identity);
                runtime.phase = DebugPhase::WaitingForClient;
            }
        }
        DebugPhase::Capturing(id) => advance_capture(world, id),
        DebugPhase::WaitingForClient => {
            if world
                .resource::<AnimationDebugRuntime>()
                .client_snapshots
                .len()
                >= 5
            {
                capture::finish_report(world);
            }
        }
        DebugPhase::Complete => {}
    }
}

fn start_animated_samples(world: &mut World) {
    let Some((identity, start, end, fps, prim_count)) =
        capture::ready_stage(world, true, None, None)
    else {
        return;
    };
    let span = end - start;
    let t0 = start + span * 0.25;
    let t1 = start + span * 0.75;
    let mut runtime = world.resource_mut::<AnimationDebugRuntime>();
    runtime.initial_identity = Some(identity);
    runtime.initial_times = Some((t0, t1, fps));
    let _ = prim_count;
    drop(runtime);
    arm_capture(world, FrameSampleId::T0, t0);
}

fn start_static_samples(world: &mut World) {
    let expected = world.resource::<AnimationDebugRuntime>().expected_identity;
    let minimum_projection_generation = world
        .resource::<AnimationDebugRuntime>()
        .expected_projection_generation;
    let Some((identity, start, end, _, _)) =
        capture::ready_stage(world, false, expected, minimum_projection_generation)
    else {
        return;
    };
    let t0 = start + (end - start) * 0.25;
    let t1 = start + (end - start) * 0.75;
    let mut runtime = world.resource_mut::<AnimationDebugRuntime>();
    runtime.static_times = Some((t0, t1));
    runtime.static_identity = Some(identity);
    drop(runtime);
    arm_capture(world, FrameSampleId::StaticT0, t0);
}

fn advance_capture(world: &mut World, id: FrameSampleId) {
    let captured = world
        .resource::<FrameSignatureDiagnostic>()
        .capture(id)
        .is_some();
    if !captured {
        return;
    }
    capture::emit_server_snapshot(world, id);
    match id {
        FrameSampleId::T0 => {
            let t1 = world
                .resource::<AnimationDebugRuntime>()
                .initial_times
                .unwrap()
                .1;
            arm_capture(world, FrameSampleId::T1, t1);
        }
        FrameSampleId::T1 => {
            let t0 = world
                .resource::<AnimationDebugRuntime>()
                .initial_times
                .unwrap()
                .0;
            arm_capture(world, FrameSampleId::T0RoundTrip, t0);
        }
        FrameSampleId::T0RoundTrip => replace_with_static_stage(world),
        FrameSampleId::StaticT0 => {
            let t1 = world
                .resource::<AnimationDebugRuntime>()
                .static_times
                .unwrap()
                .1;
            arm_capture(world, FrameSampleId::StaticT1, t1);
        }
        FrameSampleId::StaticT1 => restart_hummingbird(world),
    }
}

fn replace_with_static_stage(world: &mut World) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stages/hierarchy.usda");
    let stage = match Stage::open(path.to_string_lossy().as_ref()) {
        Ok(stage) => stage,
        Err(error) => {
            record_failure(world, format!("static control stage open failed: {error}"));
            return;
        }
    };
    if let Some(mut live) = world.get_non_send_mut::<LiveStage>() {
        live.replace_stage(stage);
        let identity = live.stage_identity();
        drop(live);
        let minimum_projection_generation = world
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .generation()
            .saturating_add(1);
        let mut runtime = world.resource_mut::<AnimationDebugRuntime>();
        runtime.expected_identity = Some(identity);
        runtime.expected_projection_generation = Some(minimum_projection_generation);
        runtime.phase = DebugPhase::WaitingForStaticStage;
    } else {
        record_failure(
            world,
            "static control replacement found no LiveStage".to_owned(),
        );
    }
}

fn restart_hummingbird(world: &mut World) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/external/hummingbird.usdz");
    let stage = match Stage::open(path.to_string_lossy().as_ref()) {
        Ok(stage) => stage,
        Err(error) => {
            record_failure(
                world,
                format!("Hummingbird restart stage open failed: {error}"),
            );
            return;
        }
    };
    if let Some(mut live) = world.get_non_send_mut::<LiveStage>() {
        live.replace_stage(stage);
        let identity = live.stage_identity();
        drop(live);
        let minimum_projection_generation = world
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .generation()
            .saturating_add(1);
        let mut runtime = world.resource_mut::<AnimationDebugRuntime>();
        runtime.expected_identity = Some(identity);
        runtime.expected_projection_generation = Some(minimum_projection_generation);
        runtime.phase = DebugPhase::WaitingForRestartStage;
    } else {
        record_failure(world, "Hummingbird restart found no LiveStage".to_owned());
    }
}

fn record_failure(world: &mut World, message: String) {
    bevy::log::error!("[animation-debug] {message}");
    world
        .resource_mut::<AnimationDebugRuntime>()
        .diagnostic_error = Some(message);
    capture::finish_report(world);
}

fn arm_capture(world: &mut World, id: FrameSampleId, time_code: f64) {
    let sequence = world
        .resource::<FrameTransportResource>()
        .0
        .next_render_sequence();
    if let Some(mut diagnostic) = world.get_resource_mut::<FrameSignatureDiagnostic>() {
        diagnostic.arm(id, sequence);
    } else {
        capture::finish_report(world);
        return;
    }
    let Some(mut clock) = world.get_resource_mut::<UsdStageTime>() else {
        capture::finish_report(world);
        return;
    };
    clock.playing = false;
    clock.seconds = (time_code - clock.start_time_code) / clock.time_codes_per_second;
    world.resource_mut::<AnimationDebugRuntime>().phase = DebugPhase::Capturing(id);
}
