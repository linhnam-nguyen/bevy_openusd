//! Scene framing, focus transitions, and authored-camera mounting.

use bevy::camera::Projection;
use bevy::prelude::*;
use usd_bevy::{Projection as UsdProjection, UsdCamera, UsdPrimRef};

use super::{
    ArcballCamera, CameraMount, CameraNavigationConfig, FlyTo, apply_rig, distance_as_f32,
    finite_positive_distance,
};
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
    cam.distance = fly.start_distance.lerp(fly.target_distance, eased);
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

/// Updates perspective clipping from active composed bounds. The renderer
/// keeps a finite far plane for culling, while user navigation itself has no
/// artificial minimum or maximum distance.
pub(crate) fn sync_adaptive_camera_clipping(
    config: Res<CameraNavigationConfig>,
    extent: Res<SceneExtent>,
    mut cameras: Query<(&GlobalTransform, &mut Projection), With<Camera3d>>,
) {
    let (center, radius, scale) = if extent.count == 0 {
        (
            Vec3::ZERO,
            config.empty_scene_radius,
            config.empty_scene_radius * 2.0,
        )
    } else {
        (
            extent.center(),
            f64::from(extent.diag()) * 0.5,
            f64::from(extent.diag()),
        )
    };

    for (transform, mut projection) in cameras.iter_mut() {
        let camera_distance = distance_between(transform.translation(), center);
        let near = adaptive_near(camera_distance, scale, &config);
        let far = adaptive_far(camera_distance, radius, near, &config);
        if let Projection::Perspective(perspective) = projection.as_mut() {
            perspective.near = near;
            perspective.far = far;
        }
    }
}

fn distance_between(a: Vec3, b: Vec3) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    let dz = f64::from(a.z) - f64::from(b.z);
    let distance = dx.hypot(dy).hypot(dz);
    if distance.is_finite() {
        distance
    } else {
        f64::MAX
    }
}

fn adaptive_near(focus_distance: f64, scene_scale: f64, config: &CameraNavigationConfig) -> f32 {
    let scale = finite_positive_distance(scene_scale, config.empty_scene_radius);
    let distance = finite_positive_distance(focus_distance, config.empty_scene_distance);
    let near = (scale * config.near_scene_scale_ratio)
        .max(distance * config.near_focus_distance_ratio)
        .max(f64::from(f32::MIN_POSITIVE));
    distance_as_f32(near)
}

fn adaptive_far(
    camera_distance: f64,
    scene_radius: f64,
    near: f32,
    config: &CameraNavigationConfig,
) -> f32 {
    let distance = finite_positive_distance(camera_distance, config.empty_scene_distance);
    let radius = finite_positive_distance(scene_radius, config.empty_scene_radius);
    let safety = (radius * config.far_safety_ratio).max(f64::from(near));
    let far = distance + radius + safety;
    let far = if far.is_finite() && far > f64::from(near) {
        far
    } else {
        f64::from(f32::MAX)
    };
    let far = distance_as_f32(far);
    if far > near { far } else { f32::MAX }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_scene_gets_a_near_plane_below_the_legacy_floor() {
        let config = CameraNavigationConfig::default();
        let near = adaptive_near(0.01, 0.02, &config);

        assert!(near.is_finite());
        assert!(near > 0.0);
        assert!(near < 0.001);
    }

    #[test]
    fn large_scene_gets_a_far_plane_beyond_the_legacy_limit() {
        let config = CameraNavigationConfig::default();
        let near = adaptive_near(100_000.0, 100_000.0, &config);
        let far = adaptive_far(100_000.0, 50_000.0, near, &config);

        assert!(near.is_finite());
        assert!(far.is_finite());
        assert!(far > near);
        assert!(far > 10_000.0);
        assert!(f64::from(far) >= 100_000.0 + 50_000.0);
    }

    #[test]
    fn empty_scene_uses_finite_ordered_clipping_fallbacks() {
        let config = CameraNavigationConfig::default();
        let near = adaptive_near(
            config.empty_scene_distance,
            config.empty_scene_radius * 2.0,
            &config,
        );
        let far = adaptive_far(
            config.empty_scene_distance,
            config.empty_scene_radius,
            near,
            &config,
        );

        assert!(near.is_finite() && near > 0.0);
        assert!(far.is_finite() && far > near);
    }
}
