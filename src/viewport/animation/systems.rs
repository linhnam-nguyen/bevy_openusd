//! Runtime animation evaluation for the current USD session.
//!
//! The systems remain registered by the viewport composition root for now so
//! this extraction preserves the existing schedule and ordering exactly.

use bevy::asset::Assets;
use bevy::prelude::*;
use usd_bevy::{UsdAsset, UsdPrimRef};

use super::{PendingAnimationClip, UsdStageTime};
use crate::viewport::session::StageHandle;

/// Advance `UsdStageTime.seconds` by the frame delta when `playing`.
/// Wraps back to the start on reaching the end so the animation loops
/// — most authored scenes are short cycles and the user can pause on
/// any frame with the timeline panel.
///
/// Also syncs `start`/`end`/`fps` from the loaded UsdAsset on first
/// sight so a fresh stage populates the clock's bounds.
/// Advances looping USD animation time while playback is active.
pub(crate) fn tick_stage_time(
    time: Res<Time>,
    mut clock: ResMut<UsdStageTime>,
    stage: Option<Res<StageHandle>>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    // Pull stage timeline metadata once after the asset lands.
    if !clock.initialized
        && let Some(stage) = stage
        && let Some(asset) = usd_assets.get(&stage.0)
    {
        clock.start_time_code = asset.start_time_code;
        clock.end_time_code = asset.end_time_code;
        clock.time_codes_per_second = asset.time_codes_per_second;
        clock.seconds = 0.0;
        // Only actual animation data makes playback meaningful. A static
        // stage may still author a non-trivial global timeline range, but
        // exposing that range as Play/Pause would repeatedly evaluate the
        // same pose and mislead the remote UI.
        let has_animation =
            asset.animated_prims.iter().next().is_some() || !asset.skel_animations.is_empty();
        if !has_animation {
            clock.end_time_code = clock.start_time_code;
        }
        clock.playing = has_animation;
        clock.initialized = true;
        info!(
            "stage time clock: start={:.2} end={:.2} fps={:.2} (duration {:.2}s) — {} animated prim(s), {} skel anim(s)",
            clock.start_time_code,
            clock.end_time_code,
            clock.time_codes_per_second,
            clock.duration_seconds(),
            asset.animated_prims.len(),
            asset.skel_animations.len()
        );
    }

    if clock.playing {
        clock.seconds += time.delta_secs_f64();
        let dur = clock.duration_seconds();
        if dur > 0.0 && clock.seconds >= dur {
            // Wrap immediately — no hold at the endpoint. Carry the
            // overshoot so playback stays smooth across the seam.
            clock.seconds = clock.seconds.rem_euclid(dur);
        }
    }
}

/// Re-evaluate animated xformOps for every prim in
/// `UsdAsset::animated_prims` and write the resulting `Transform`. Runs
/// every frame — cheap because only prims with authored timeSamples
/// are touched (the rest stay static at their load-time Transform).
/// Evaluates transform time samples at the current stage time code.
pub(crate) fn evaluate_animated_prims(
    clock: Res<UsdStageTime>,
    stage: Option<Res<StageHandle>>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut prims: Query<(&UsdPrimRef, &mut Transform)>,
) {
    // A paused clock keeps the current pose. Re-evaluate once on the frame
    // where the clock changes (including the SetPlayback command), but do not
    // resample the same USD time code every render frame afterwards.
    if !clock.is_changed() {
        return;
    }
    let Some(stage) = stage else { return };
    let Some(asset) = usd_assets.get(&stage.0) else {
        return;
    };
    if asset.animated_prims.is_empty() {
        return;
    }
    let tc = clock.current_time_code();
    use usd_schema::anim::eval_scalar_track;

    for (prim_ref, mut tr) in prims.iter_mut() {
        let Some(record) = asset.animated_prims.get(&prim_ref.path) else {
            continue;
        };
        // Single-axis rotates: if present, overwrite Transform.rotation
        // with a quat for the sampled degree value. We scope to the one
        // axis the prim authored (non-overlapping per USD spec).
        // `eval_scalar_track` dispatches linear vs held based on the
        // authored `interpolation` metadata.
        if let Some(track) = &record.rotate_y
            && let Some(deg) = eval_scalar_track(track, tc)
        {
            tr.rotation = bevy::math::Quat::from_rotation_y(deg.to_radians());
        }
        if let Some(track) = &record.rotate_x
            && let Some(deg) = eval_scalar_track(track, tc)
        {
            tr.rotation = bevy::math::Quat::from_rotation_x(deg.to_radians());
        }
        if let Some(track) = &record.rotate_z
            && let Some(deg) = eval_scalar_track(track, tc)
        {
            tr.rotation = bevy::math::Quat::from_rotation_z(deg.to_radians());
        }
    }
}

/// Per-frame driver for `UsdSkelAnimation`. For each `UsdSkelAnimDriver`
/// component (one per SkelRoot with a matched sidecar animation):
///
/// 1. Evaluate the animation at the current stage time → one
///    `EvaluatedJoint` per channel.
/// 2. Map each channel to its skeleton joint entity (already
///    pre-resolved at load time and stored in
///    `driver.joint_entities[i]`) and apply the evaluated translation
///    / rotation / scale to that joint's local `Transform`.
///
/// Skips the eval when no driver is present (no animation loaded), so
/// the cost on non-animated scenes is one `Query::iter()` per frame.
/// Switches the active skeletal clip without reloading the USD stage.
pub(crate) fn apply_live_animation_clip(
    mut pending: ResMut<PendingAnimationClip>,
    stage: Option<Res<StageHandle>>,
    assets: Res<Assets<UsdAsset>>,
    mut drivers: Query<&mut usd_bevy::prim_ref::UsdSkelAnimDriver>,
    mut clock: ResMut<UsdStageTime>,
) {
    let Some(clip) = pending.name.take() else {
        return;
    };
    let Some(stage) = stage else {
        pending.name = Some(clip);
        return;
    };
    let Some(asset) = assets.get(&stage.0) else {
        pending.name = Some(clip);
        return;
    };
    let Some(anim) = asset.skel_animations.get(&clip) else {
        warn!("animation switch: clip {clip:?} was not found in loaded UsdAsset");
        return;
    };

    let mut updated = 0usize;
    for mut driver in drivers.iter_mut() {
        let anim_to_skel: Vec<Option<Entity>> = anim
            .joints
            .iter()
            .map(|jp| {
                driver
                    .skeleton_joints
                    .iter()
                    .position(|sp| sp == jp)
                    .and_then(|i| driver.skeleton_joint_entities.get(i).copied().flatten())
            })
            .collect();
        let mapped = anim_to_skel.iter().filter(|e| e.is_some()).count();
        if mapped == 0 && !anim.joints.is_empty() {
            warn!(
                "animation switch: clip {clip:?} did not map to this skeleton ({} channels)",
                anim.joints.len()
            );
            continue;
        }

        driver.anim_name = anim.prim_name.clone();
        driver.joint_entities = anim_to_skel;
        driver.translations = anim
            .translations
            .iter()
            .map(|(t, v)| (t.0, v.clone()))
            .collect();
        driver.rotations = anim
            .rotations
            .iter()
            .map(|(t, v)| (t.0, v.clone()))
            .collect();
        driver.scales = anim.scales.iter().map(|(t, v)| (t.0, v.clone())).collect();
        driver.blend_shape_names = anim.blend_shapes.clone();
        driver.blend_shape_weights = anim
            .blend_shape_weights
            .iter()
            .map(|(t, v)| (t.0, v.clone()))
            .collect();
        driver.quat_xyzw_order = detect_quat_xyzw_order(anim);
        updated += 1;
        info!(
            "animation switch: live-swapped to {} (mapped {mapped}/{} channels)",
            anim.prim_name,
            anim.joints.len()
        );
    }

    if updated > 0 {
        if let Some((start, end)) = skel_anim_time_range(anim) {
            clock.start_time_code = start;
            clock.end_time_code = end.max(start + 1.0);
            clock.seconds = 0.0;
            clock.initialized = true;
        } else {
            clock.seconds = 0.0;
        }
    } else {
        warn!("animation switch: no live UsdSkelAnimDriver accepted clip {clip:?}");
    }
}

/// Detects whether a parsed skeletal animation stores quaternions in XYZW order.
fn detect_quat_xyzw_order(anim: &usd_schema::skel_anim_text::ReadSkelAnimText) -> bool {
    let mut sum_abs_first = 0.0f32;
    let mut sum_abs_last = 0.0f32;
    let mut samples = 0usize;
    if let Some((_, first_rot)) = anim.rotations.iter().next() {
        for q in first_rot {
            sum_abs_first += q[0].abs();
            sum_abs_last += q[3].abs();
            samples += 1;
        }
    }
    samples > 0 && sum_abs_last > sum_abs_first
}

/// Extracts the inclusive authored time-code range from a skeletal clip.
fn skel_anim_time_range(anim: &usd_schema::skel_anim_text::ReadSkelAnimText) -> Option<(f64, f64)> {
    let mut start = f64::INFINITY;
    let mut end = f64::NEG_INFINITY;
    for t in anim
        .translations
        .iter()
        .map(|(t, _)| t.0)
        .chain(anim.rotations.iter().map(|(t, _)| t.0))
        .chain(anim.scales.iter().map(|(t, _)| t.0))
        .chain(anim.blend_shape_weights.iter().map(|(t, _)| t.0))
    {
        start = start.min(t);
        end = end.max(t);
    }
    start.is_finite().then_some((start, end))
}

/// Samples authored skeletal animation and updates the live joint poses.
pub(crate) fn drive_skel_animations(
    clock: Res<UsdStageTime>,
    drivers: Query<&usd_bevy::prim_ref::UsdSkelAnimDriver>,
    mut joints: Query<&mut Transform>,
    mut diag_emitted: Local<bool>,
    mut tick: Local<u32>,
) {
    if !clock.is_changed() {
        return;
    }
    let tc = clock.current_time_code();
    *tick += 1;
    for driver in drivers.iter() {
        let evaluated = usd_bevy::skel_anim::evaluate(driver, tc);
        let mut hits = 0usize;
        let mut misses = 0usize;
        // Sample joint 0's local Transform translation BEFORE applying
        // — to verify driver actually changes values across frames.
        let probe_je = driver.joint_entities.iter().skip(10).find_map(|e| *e);
        let before: Option<(bevy::math::Vec3, bevy::math::Quat)> =
            probe_je.and_then(|je| joints.get(je).ok().map(|t| (t.translation, t.rotation)));
        for (channel_ix, joint_entity) in driver.joint_entities.iter().enumerate() {
            let Some(je) = joint_entity else {
                misses += 1;
                continue;
            };
            let Ok(mut tr) = joints.get_mut(*je) else {
                misses += 1;
                continue;
            };
            evaluated[channel_ix].apply(&mut tr);
            hits += 1;
        }
        let after: Option<(bevy::math::Vec3, bevy::math::Quat)> =
            probe_je.and_then(|je| joints.get(je).ok().map(|t| (t.translation, t.rotation)));
        if !*diag_emitted && (hits > 0 || misses > 0) {
            info!(
                "skel anim: first-tick wrote {hits}/{} joints (missed {misses}); probe before={before:?} after={after:?} tc={tc:.2}",
                driver.joint_entities.len()
            );
            *diag_emitted = true;
        } else if *tick % 30 == 0 {
            info!("skel anim tick={} tc={tc:.2} probe={after:?}", *tick);
        }
    }
}

/// Per-frame driver for blend-shape weights. Reads
/// `UsdSkelAnimDriver`'s blendShapeWeights, looks up each mesh's
/// per-target name in the anim's `blend_shape_names`, and writes
/// the matching weight into `MeshMorphWeights`. Missing names get
/// weight 0.
///
/// For multi-skel scenes this picks the first driver — single-rig
/// is the common case (HumanFemale, etc.).
/// Evaluates blend-shape animation samples and writes current mesh weights.
pub(crate) fn drive_blend_shape_weights(
    clock: Res<UsdStageTime>,
    drivers: Query<&usd_bevy::prim_ref::UsdSkelAnimDriver>,
    mut meshes: Query<(
        &usd_bevy::prim_ref::UsdBlendShapeBinding,
        &mut bevy::mesh::morph::MeshMorphWeights,
    )>,
    mut eval_diag_emitted: Local<bool>,
    mut mapping_diag_emitted: Local<bool>,
) {
    if !clock.is_changed() {
        return;
    }
    let Some(driver) = drivers.iter().next() else {
        return;
    };
    let tc = clock.current_time_code();
    let evaluated = usd_bevy::skel_anim::evaluate_blend_shapes(driver, tc);
    if !*eval_diag_emitted {
        let nz: usize = evaluated.iter().filter(|w| w.abs() > 1e-4).count();
        let mx: f32 = evaluated.iter().map(|w| w.abs()).fold(0.0_f32, f32::max);
        info!(
            "blend anim: evaluated {} weights at tc={:.1}, nonzero={nz}, max={mx:.3}",
            evaluated.len(),
            tc
        );
        *eval_diag_emitted = true;
    }
    if evaluated.is_empty() {
        return;
    }
    // Build a lookup table from blend-shape name → weight index
    // (one-time per-frame cost; the names list is short relative to
    // mesh count).
    let name_to_ix: std::collections::HashMap<&str, usize> = driver
        .blend_shape_names
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    // Debug override: BEVY_OPENUSD_BLEND_DEBUG=1 forces all morph
    // weights to 1.0 so we can verify the GPU morph path is wired
    // independently of the anim's weight values. If meshes change
    // shape with this set but not without, the issue is the anim →
    // weight mapping; if they don't change either, the morph
    // rendering itself isn't applying.
    let force_all = std::env::var("BEVY_OPENUSD_BLEND_DEBUG")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    let mut total_meshes = 0usize;
    let mut total_targets = 0usize;
    for (binding, mut weights) in meshes.iter_mut() {
        total_meshes += 1;
        let bevy::mesh::morph::MeshMorphWeights::Value { weights: buf } = &mut *weights else {
            continue;
        };
        for (slot, name) in binding.names.iter().enumerate() {
            if slot >= buf.len() {
                break;
            }
            buf[slot] = if force_all {
                1.0
            } else {
                name_to_ix
                    .get(name.as_str())
                    .and_then(|i| evaluated.get(*i))
                    .copied()
                    .unwrap_or(0.0)
            };
            total_targets += 1;
        }
    }
    if !*mapping_diag_emitted && total_meshes > 0 {
        // Sample first weight buffer to confirm we're writing
        // non-zero values + the underlying mesh asset has morph
        // targets attached.
        let mut sample_nonzero = 0;
        let mut sample_max = 0.0f32;
        let mut sample_buf_len = 0;
        for (_, weights) in meshes.iter().take(1) {
            let bevy::mesh::morph::MeshMorphWeights::Value { weights } = weights else {
                continue;
            };
            sample_buf_len = weights.len();
            for w in weights {
                if w.abs() > 1e-4 {
                    sample_nonzero += 1;
                }
                sample_max = sample_max.max(w.abs());
            }
        }
        info!(
            "blend anim: drove {total_meshes} meshes, {total_targets} targets across {} anim channels; sample buf len={sample_buf_len} nonzero={sample_nonzero} max={sample_max:.3}",
            evaluated.len()
        );
        *mapping_diag_emitted = true;
    }
}
