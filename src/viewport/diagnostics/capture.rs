use bevy::prelude::World;
use usd_bevy::{AnimatedPrims, LiveStage, ProgressiveProjectionState, ProjectionReadiness};
use viewport_protocol::{AnimationDebugSnapshot, ViewportEvent, ViewportEventEnvelope};

use super::{AnimationDebugRuntime, FrameSampleId, UsdStageTime};
use crate::viewport::{
    api::ViewportEventOutbox, transport::frame_signature::FrameSignatureDiagnostic,
};

pub(super) fn ready_stage(
    world: &World,
    expect_animated: bool,
    expected_identity: Option<(u64, u64)>,
    minimum_projection_generation: Option<u64>,
) -> Option<((u64, u64), f64, f64, f64, u32)> {
    let live = world.get_non_send::<LiveStage>()?;
    let identity = live.stage_identity();
    if expected_identity.is_some_and(|expected| expected != identity) {
        return None;
    }
    let projection = world.get_resource::<ProgressiveProjectionState>()?;
    if projection.readiness() != ProjectionReadiness::Ready
        || projection.session_id() != Some(identity.0)
        || minimum_projection_generation.is_some_and(|minimum| projection.generation() < minimum)
    {
        return None;
    }
    let animated = world.get_resource::<AnimatedPrims>()?;
    let count = u32::try_from(animated.0.len()).ok()?;
    if (count > 0) != expect_animated {
        return None;
    }
    let start = live.stage.start_time_code();
    let end = live.stage.end_time_code();
    let fps = live.stage.time_codes_per_second();
    if fps <= 0.0 || (expect_animated && end <= start) {
        return None;
    }
    if world.resource::<UsdStageTime>().stage_identity() != Some(identity) {
        return None;
    }
    Some((identity, start, end.max(start + 1.0), fps, count))
}

pub(super) fn emit_server_snapshot(world: &mut World, id: FrameSampleId) {
    let Some(capture) = world
        .resource::<FrameSignatureDiagnostic>()
        .capture(id)
        .cloned()
    else {
        return;
    };
    let Some(live) = world.get_non_send::<LiveStage>() else {
        return;
    };
    let identity = live.stage_identity();
    let stage_ready = world
        .get_resource::<ProgressiveProjectionState>()
        .is_some_and(|state| {
            state.readiness() == ProjectionReadiness::Ready
                && state.session_id() == Some(identity.0)
                && world.resource::<UsdStageTime>().stage_identity() == Some(identity)
        });
    let snapshot = AnimationDebugSnapshot {
        sample: Some(super::report::protocol_sample_id(id)),
        stage_session_id: Some(identity.0),
        stage_generation: Some(identity.1),
        stage_ready: Some(stage_ready),
        animated_prim_count: world
            .get_resource::<AnimatedPrims>()
            .and_then(|animated| u32::try_from(animated.0.len()).ok()),
        time_code: world
            .get_resource::<usd_bevy::StageTime>()
            .map(|time| time.current),
        transform_hash: super::report::transform_hash(world),
        render_sequence: Some(capture.sequence),
        render_hash: Some(capture.sample.hash),
        render_mean_luma: Some(capture.sample.mean_luma),
        frames_received: None,
        frames_decoded: None,
        playback_total_frames: None,
        delivery_frames: None,
        presented_frames: None,
        presentation_proof: None,
        client_content_signature_supported: None,
        client_content_hash: None,
    };
    world
        .resource_mut::<AnimationDebugRuntime>()
        .server_snapshots
        .push(snapshot);
    if let Some(mut outbox) = world.get_resource_mut::<ViewportEventOutbox>() {
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::AnimationDebugServerSample { snapshot },
        ));
    }
}

pub(super) fn finish_report(world: &mut World) {
    let path = world
        .resource::<AnimationDebugRuntime>()
        .output_path
        .clone();
    let report = super::report::build_report(world);
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(path, bytes);
        }
    }
    world.resource_mut::<AnimationDebugRuntime>().phase = super::DebugPhase::Complete;
    world.write_message(bevy::app::AppExit::Success);
}
