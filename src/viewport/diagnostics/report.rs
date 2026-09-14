use crate::viewport::transport::frame_signature::CapturedFrameSignature;
use bevy::prelude::{Transform, World};
use viewport_protocol::AnimationDebugSampleId;

use super::{
    AnimationDebugRuntime, FrameSampleId, FrameSignatureDiagnostic, HUMMINGBIRD_MAX_REPEAT_MAD,
    HUMMINGBIRD_MIN_MAD, STATIC_MAX_MAD,
};

pub(super) fn protocol_sample_id(id: FrameSampleId) -> AnimationDebugSampleId {
    match id {
        FrameSampleId::T0 => AnimationDebugSampleId::T0,
        FrameSampleId::T1 => AnimationDebugSampleId::T1,
        FrameSampleId::T0RoundTrip => AnimationDebugSampleId::T0RoundTrip,
        FrameSampleId::StaticT0 => AnimationDebugSampleId::StaticT0,
        FrameSampleId::StaticT1 => AnimationDebugSampleId::StaticT1,
    }
}

pub(super) fn build_report(world: &mut World) -> serde_json::Value {
    let runtime = world.resource::<AnimationDebugRuntime>();
    let captures = world
        .resource::<FrameSignatureDiagnostic>()
        .captures()
        .to_vec();
    let samples = runtime
        .server_snapshots
        .iter()
        .map(|snapshot| {
            let mad_from_previous = snapshot.sample.and_then(|sample| {
                let id = frame_sample_id(sample);
                let index = captures.iter().position(|capture| capture.id == id)?;
                (index > 0).then(|| captures[index].sample.mad(&captures[index - 1].sample))
            });
            serde_json::json!({
                "sample": snapshot.sample,
                "stage_session_id": snapshot.stage_session_id,
                "stage_generation": snapshot.stage_generation,
                "stage_ready": snapshot.stage_ready,
                "animated_prim_count": snapshot.animated_prim_count,
                "time_code": snapshot.time_code,
                "render_sequence": snapshot.render_sequence,
                "render_hash": snapshot.render_hash.map(|hash| format!("{hash:016x}")),
                "render_mean_luma": snapshot.render_mean_luma,
                "transform_hash": snapshot.transform_hash.map(|hash| format!("{hash:016x}")),
                "mad_luma_from_previous_capture": mad_from_previous,
            })
        })
        .collect::<Vec<_>>();
    let identity = runtime.initial_identity.unwrap_or_default();
    let (t0, t1, fps) = runtime.initial_times.unwrap_or_default();
    let pairwise = pairwise_mad(&captures);
    let server = server_evidence(&runtime.server_snapshots);
    let client = client_evidence(&runtime.client_snapshots);
    let failure_layer = classify_failure(runtime, &server, &pairwise, &client);
    let stage_ready = runtime.initial_identity.is_some()
        && runtime.static_identity.is_some()
        && runtime.restart_identity.is_some()
        && runtime
            .server_snapshots
            .iter()
            .all(|sample| sample.stage_ready == Some(true));
    serde_json::json!({
        "schema_version": 2,
        "fixture": "hummingbird.usdz",
        "static_control": "hierarchy.usda",
        "stage_identity": {"session_id": identity.0, "generation": identity.1},
        "stage_transitions": {
            "initial": runtime.initial_identity.map(identity_value),
            "static_control": runtime.static_identity.map(identity_value),
            "hummingbird_restart": runtime.restart_identity.map(identity_value),
        },
        "stage_ready": stage_ready,
        "diagnostic_error": runtime.diagnostic_error,
        "animated_prim_count": runtime.server_snapshots.first()
            .and_then(|sample| sample.animated_prim_count),
        "time_codes_per_second": fps,
        "t0": t0,
        "t1": t1,
        "stage_time_advanced": t1 > t0,
        "samples": samples,
        "server_evidence": server,
        "pairwise_render_mad_luma": pairwise,
        "client_samples": runtime.client_snapshots,
        "client_evidence": client,
        "thresholds": {
            "hummingbird_min_mad": HUMMINGBIRD_MIN_MAD,
            "hummingbird_max_repeat_mad": HUMMINGBIRD_MAX_REPEAT_MAD,
            "static_max_mad": STATIC_MAX_MAD,
            "calibration": {
                "basis": "documented_multi_run_native_headless_fixture_calibration",
                "current_run_observed_hummingbird_mad": pairwise["hummingbird_t0_t1"],
                "current_run_observed_repeat_mad": pairwise["hummingbird_t0_round_trip"],
                "current_run_observed_static_mad": pairwise["static_t0_t1"],
                "frozen_thresholds": {
                    "motion_floor": HUMMINGBIRD_MIN_MAD,
                    "repeatability_ceiling": HUMMINGBIRD_MAX_REPEAT_MAD,
                    "static_noise_ceiling": STATIC_MAX_MAD,
                },
            },
        },
        "pass": failure_layer.is_none(),
        "failure_layer": failure_layer,
    })
}

fn frame_sample_id(id: AnimationDebugSampleId) -> FrameSampleId {
    match id {
        AnimationDebugSampleId::T0 => FrameSampleId::T0,
        AnimationDebugSampleId::T1 => FrameSampleId::T1,
        AnimationDebugSampleId::T0RoundTrip => FrameSampleId::T0RoundTrip,
        AnimationDebugSampleId::StaticT0 => FrameSampleId::StaticT0,
        AnimationDebugSampleId::StaticT1 => FrameSampleId::StaticT1,
    }
}

fn pairwise_mad(captures: &[CapturedFrameSignature]) -> serde_json::Value {
    let mad = |left: FrameSampleId, right: FrameSampleId| {
        let left = captures.iter().find(|capture| capture.id == left)?;
        let right = captures.iter().find(|capture| capture.id == right)?;
        Some(left.sample.mad(&right.sample))
    };
    serde_json::json!({
        "hummingbird_t0_t1": mad(FrameSampleId::T0, FrameSampleId::T1),
        "hummingbird_t0_round_trip": mad(FrameSampleId::T0, FrameSampleId::T0RoundTrip),
        "static_t0_t1": mad(FrameSampleId::StaticT0, FrameSampleId::StaticT1),
    })
}

fn server_evidence(snapshots: &[viewport_protocol::AnimationDebugSnapshot]) -> serde_json::Value {
    let find =
        |id: AnimationDebugSampleId| snapshots.iter().find(|sample| sample.sample == Some(id));
    let t0 = find(AnimationDebugSampleId::T0);
    let t1 = find(AnimationDebugSampleId::T1);
    let round_trip = find(AnimationDebugSampleId::T0RoundTrip);
    let static_t0 = find(AnimationDebugSampleId::StaticT0);
    let static_t1 = find(AnimationDebugSampleId::StaticT1);
    serde_json::json!({
        "t0": t0,
        "t1": t1,
        "t0_round_trip": round_trip,
        "static_t0": static_t0,
        "static_t1": static_t1,
        "hash_t0_differs_from_t1": t0.zip(t1).and_then(|(a, b)| Some(a.render_hash? != b.render_hash?)),
        "sequence_t1_after_t0": t0.zip(t1).and_then(|(a, b)| Some(b.render_sequence? > a.render_sequence?)),
        "sequence_round_trip_after_t1": t1.zip(round_trip).and_then(|(a, b)| Some(b.render_sequence? > a.render_sequence?)),
        "static_sequence_advances": static_t0.zip(static_t1).and_then(|(a, b)| Some(b.render_sequence? > a.render_sequence?)),
    })
}

fn client_evidence(snapshots: &[viewport_protocol::AnimationDebugSnapshot]) -> serde_json::Value {
    let presented = snapshots
        .iter()
        .filter_map(|sample| sample.presented_frames);
    let first_presented = presented.clone().next();
    let last_presented = presented.last();
    let decoded = snapshots.iter().filter_map(|sample| sample.frames_decoded);
    serde_json::json!({
        "sample_count": snapshots.len(),
        "presented_delta": first_presented.zip(last_presented).map(|(first, last)| last.saturating_sub(first)),
        "decoded_delta": decoded.clone().next().zip(decoded.last()).map(|(first, last)| last.saturating_sub(first)),
        "proofs": snapshots.iter().filter_map(|sample| sample.presentation_proof).collect::<Vec<_>>(),
        "content_signature_supported": snapshots.iter().find_map(|sample| sample.client_content_signature_supported),
        "content_hashes": snapshots.iter().filter_map(|sample| sample.client_content_hash).map(|hash| format!("{hash:016x}")).collect::<Vec<_>>(),
    })
}

fn classify_failure(
    runtime: &AnimationDebugRuntime,
    server: &serde_json::Value,
    pairwise: &serde_json::Value,
    client: &serde_json::Value,
) -> Option<&'static str> {
    if runtime.initial_identity.is_none()
        || runtime
            .server_snapshots
            .iter()
            .any(|sample| sample.stage_ready != Some(true))
    {
        return Some("STAGE_NOT_READY");
    }
    if runtime
        .server_snapshots
        .first()
        .and_then(|sample| sample.animated_prim_count)
        == Some(0)
    {
        return Some("NO_AUTHORED_ANIMATION");
    }
    if runtime.initial_times.is_none() || runtime.initial_times.is_some_and(|(t0, t1, _)| t1 <= t0)
    {
        return Some("CLOCK_NOT_ADVANCING");
    }
    if server["t0"].get("transform_hash") == server["t1"].get("transform_hash") {
        return Some("ANIMATION_EVALUATION_STATIC");
    }
    if pairwise["hummingbird_t0_t1"].is_null()
        || pairwise["hummingbird_t0_t1"].as_f64().unwrap_or(0.0) < HUMMINGBIRD_MIN_MAD
    {
        return Some("HEADLESS_RENDER_STATIC");
    }
    if server["sequence_t1_after_t0"] != true || server["sequence_round_trip_after_t1"] != true {
        return Some("VIDEO_FRAME_CAPTURE_STALLED");
    }
    if server["static_sequence_advances"] != true
        || pairwise["static_t0_t1"].is_null()
        || pairwise["static_t0_t1"].as_f64().unwrap_or(f64::INFINITY) > STATIC_MAX_MAD
    {
        return Some("HEADLESS_RENDER_STATIC");
    }
    if pairwise["hummingbird_t0_round_trip"]
        .as_f64()
        .is_none_or(|mad| mad > HUMMINGBIRD_MAX_REPEAT_MAD)
    {
        return Some("NON_DETERMINISTIC_ROUND_TRIP");
    }
    if runtime.client_snapshots.is_empty() {
        return Some("ENCODE_OR_WEBRTC_STALLED");
    }
    if client["decoded_delta"].as_u64().unwrap_or(0) == 0 {
        return Some("CLIENT_DECODE_STALLED");
    }
    if client["presented_delta"].as_u64().unwrap_or(0) == 0 {
        return Some("CLIENT_PRESENTATION_STALLED");
    }
    client_content_failure(client)
}

fn client_content_failure(client: &serde_json::Value) -> Option<&'static str> {
    if client["content_signature_supported"].as_bool() != Some(true) {
        return None;
    }
    let hashes = client["content_hashes"].as_array()?;
    (hashes.len() >= 2 && hashes.windows(2).all(|pair| pair[0] == pair[1]))
        .then_some("CLIENT_CONTENT_STATIC")
}

fn identity_value(identity: (u64, u64)) -> serde_json::Value {
    serde_json::json!({"session_id": identity.0, "generation": identity.1})
}

pub(super) fn transform_hash(world: &mut World) -> Option<u64> {
    let mut samples = Vec::new();
    let mut paths = world
        .resource::<usd_bevy::AnimatedPrims>()
        .0
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    let map = world.resource::<usd_bevy::PrimEntities>();
    let path_store = world.resource::<usd_bevy::PathStore>();
    for path in paths {
        if let Some(entity) = map.entity(path_store, &path)
            && let Some(transform) = world.get::<Transform>(entity)
        {
            samples.push((path, *transform));
        }
    }
    let mut joints = world.query::<(&usd_bevy::route::skel::UsdJoint, &Transform)>();
    samples.extend(
        joints
            .iter(world)
            .map(|(joint, transform)| (format!("joint:{}", joint.path), *transform)),
    );
    samples.sort_by(|(left, _), (right, _)| left.cmp(right));
    if samples.is_empty() {
        return None;
    }
    let mut hash = 14_695_981_039_346_656_037_u64;
    for (path, transform) in samples {
        update_hash(&mut hash, &(path.len() as u64).to_le_bytes());
        update_hash(&mut hash, path.as_bytes());
        for value in transform
            .translation
            .to_array()
            .into_iter()
            .chain(transform.rotation.to_array())
            .chain(transform.scale.to_array())
        {
            update_hash(
                &mut hash,
                &((f64::from(value) * 1_000_000.0).round() as i64).to_le_bytes(),
            );
        }
    }
    Some(hash)
}

fn update_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(1_099_511_628_211);
    }
}

#[cfg(test)]
mod tests {
    use super::client_content_failure;

    #[test]
    fn unsupported_client_content_probe_is_informational() {
        assert_eq!(
            client_content_failure(&serde_json::json!({
                "content_signature_supported": false,
                "content_hashes": []
            })),
            None
        );
        assert_eq!(
            client_content_failure(&serde_json::json!({
                "content_hashes": []
            })),
            None
        );
    }

    #[test]
    fn supported_static_client_content_is_classified() {
        assert_eq!(
            client_content_failure(&serde_json::json!({
                "content_signature_supported": true,
                "content_hashes": ["deadbeef", "deadbeef"]
            })),
            Some("CLIENT_CONTENT_STATIC")
        );
    }
}
