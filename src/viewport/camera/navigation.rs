//! Scene framing, focus transitions, and authored-camera mounting.

use bevy::prelude::*;
use usd_bevy::{Projection as UsdProjection, UsdCamera, UsdPrimRef};

use super::{ArcballCamera, CameraMount, FlyTo, apply_rig};
use crate::viewport::scene::visualization::SceneExtent;

/// Recenter the arcball on whatever the USD projection spawned the moment
/// enough prims show up to have a valid bounding box. Runs exactly once so
/// the user can still orbit / pan afterwards.
/// Frames the arcball camera around the scene after its first materialization.
pub(crate) fn fit_camera_once(
    extent: Res<SceneExtent>,
    mut cameras: Query<&mut ArcballCamera>,
    mut done: Local<bool>,
    mut wait_ticks: Local<u32>,
    mut last_diag: Local<f32>,
    prims: Query<(), With<UsdPrimRef>>,
) {
    if *done || extent.count == 0 || prims.iter().count() == 0 {
        return;
    }
    // Wait for the extent to stabilize. Bevy populates `Aabb` components
    // for skinned meshes only after the mesh asset is uploaded, which
    // happens a few frames after the prim entities first spawn. If we
    // frame the camera on the first available extent, assets that
    // don't author per-prim `extent` metadata (Apple's chameleon is
    // the canonical case) compute a 1cm scene diagonal from prim
    // origins alone — the camera zooms in to a point and the actual
    // mesh sits behind the camera.
    let diag = extent.diag();
    *wait_ticks += 1;
    if *wait_ticks < 60 && diag > *last_diag * 1.05 {
        // Extent is still growing — keep waiting.
        *last_diag = diag;
        return;
    }
    let Ok(mut cam) = cameras.single_mut() else {
        return;
    };
    cam.focus = extent.centre();
    // Skinned scenes (Apple AR / UsdSkel chameleon) don't author
    // per-prim extent metadata and Bevy doesn't populate `Aabb` for
    // skinned meshes (skinning happens in render, after the
    // CPU-side extent compute), so the diag we see can be ~0
    // even after waiting. Use a 2m fallback radius so the camera
    // at least frames a region the mesh likely fits in — the user
    // can scroll-out from there if the actual asset is bigger.
    let effective = diag.max(2.0);
    cam.distance = effective * 1.1;
    cam.zoom_target = cam.distance as f64;
    cam.max_distance = cam.distance.max(cam.max_distance) * 4.0;
    // Scale the zoom-in clamp to the scene size: 0.005% of the diagonal
    // floors at 1mm so a 100m greenhouse can still be inspected at
    // millimetre detail and a 30cm asset doesn't refuse to zoom past 20cm
    // (the original 0.2m default). Matches the camera-distance scaling
    // above so dolly stops just before the mesh.
    cam.min_distance = (effective * 0.00005).max(0.001);
    *done = true;
    info!(
        "camera framed on scene: focus={:?}, diag={:.2} m (effective={:.2} m), {} prims (waited {} ticks)",
        cam.focus, diag, effective, extent.count, *wait_ticks
    );
}

/// Lerp the arcball's focus + distance toward the last-requested
/// target. Zero `remaining` is the sentinel "no tween in flight".
/// Advances the active focus-and-distance tween used by tree navigation.
pub(crate) fn apply_fly_to(
    time: Res<Time>,
    mut fly: ResMut<FlyTo>,
    mut cameras: Query<(&mut Transform, &mut ArcballCamera)>,
) {
    if fly.remaining <= 0.0 {
        return;
    }
    let Ok((mut transform, mut cam)) = cameras.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(1.0 / 30.0);
    fly.remaining = (fly.remaining - dt).max(0.0);
    let progress = if fly.duration > 0.0 {
        1.0 - (fly.remaining / fly.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    // Cosine ease-out.
    let eased = (progress * core::f32::consts::FRAC_PI_2).sin();

    cam.focus = fly.start_focus.lerp(fly.target_focus, eased);
    cam.distance = fly
        .start_distance
        .lerp(fly.target_distance, eased)
        .max(cam.min_distance);
    cam.zoom_target = cam.distance as f64;

    // Bookmark restores set start/target yaw + elevation; pick the
    // shortest angular path so a 359° → 1° tween doesn't sweep the
    // long way round.
    if let (Some(sy), Some(ty)) = (fly.start_yaw, fly.target_yaw) {
        cam.yaw = lerp_angle(sy, ty, eased);
    }
    if let (Some(se), Some(te)) = (fly.start_elevation, fly.target_elevation) {
        cam.elevation = se + (te - se) * eased;
    }
    apply_rig(&cam, &mut transform);
}

/// Interpolates angles along the shortest wrapped path.
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let two_pi = core::f32::consts::TAU;
    let mut delta = (b - a) % two_pi;
    if delta > core::f32::consts::PI {
        delta -= two_pi;
    } else if delta < -core::f32::consts::PI {
        delta += two_pi;
    }
    a + delta * t
}

/// When `CameraMount::Mounted { prim_path }` is set, copy the USD camera
/// prim's `GlobalTransform` + projection onto the live `Camera3d` every
/// frame. Goes quiet in `CameraMount::Arcball` mode so the arcball runs
/// unopposed.
/// Copies transform and projection from the selected authored USD camera.
pub(crate) fn follow_mounted_camera(
    mount: Res<CameraMount>,
    prims: Query<(&UsdPrimRef, &GlobalTransform, &UsdProjection), With<UsdCamera>>,
    mut cameras: Query<
        (&mut Transform, &mut bevy::camera::Projection),
        (With<Camera3d>, Without<UsdCamera>),
    >,
) {
    let CameraMount::Mounted { prim_path } = &*mount else {
        return;
    };
    // Find the entity whose UsdPrimRef matches the mounted camera so we
    // can read its world transform. Every geom + xform prim gets a
    // UsdPrimRef, including Camera prims.
    let Some((_, gt, authored_projection)) = prims.iter().find(|(pr, _, _)| pr.path == *prim_path)
    else {
        return;
    };

    let Ok((mut tr, mut proj)) = cameras.single_mut() else {
        return;
    };

    let world = gt.compute_transform();
    tr.translation = world.translation;
    tr.rotation = world.rotation;
    // Leave scale alone — distorting cameras via scale is a footgun.

    *proj = authored_projection.clone();
}
