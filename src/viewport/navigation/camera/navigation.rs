//! Scene framing lifecycle, focus transitions, authored-camera mounting, and
//! single-authority adaptive camera clipping.

use bevy::camera::{Camera, CameraProjection, Projection};
use bevy::camera_controller::pan_orbit_camera::controller::component::PanOrbitCamera;
use bevy::prelude::*;
use usd_bevy::{Projection as UsdProjection, UsdCamera, UsdPrimRef};

use super::framing::{
    FramingOutcome, apply_framing_fit, effective_viewport_size, frame_intent_for_camera,
    framing_for_scene_extent, validated_orthographic_scale_area,
};
use super::{
    ArcballCamera, CameraMount, CameraNavigationConfig, FlyTo, MAX_CAMERA_DISTANCE,
    MIN_CAMERA_DISTANCE, apply_rig, distance_as_f32, finite_positive_distance,
    sanitize_camera_distance, sync_pan_orbit_from_arcball,
};
use crate::viewport::rendering::scene::visualization::SceneExtent;

/// Minimum near clipping plane kept strictly positive for perspective depth
/// projection.
pub(super) const MIN_NEAR_PLANE: f64 = 1.0e-8;

/// Local far-plane multiple of camera-to-focus depth so nearby context around
/// the focus target remains visible even in an empty scene.
const FAR_LOCAL_DEPTH_MULTIPLIER: f64 = 100.0;

/// Recenter the arcball on whatever the USD projection spawned the moment
/// enough prims show up to have a valid bounding box. Runs exactly once so
/// the user can still orbit / pan afterwards.
/// Frames the arcball camera around the scene after its first materialization.
pub(crate) fn fit_camera_once(
    mount: Res<CameraMount>,
    extent: Res<SceneExtent>,
    config: Res<CameraNavigationConfig>,
    mut cameras: Query<
        (
            &mut Transform,
            &mut ArcballCamera,
            Option<&mut PanOrbitCamera>,
            &mut Projection,
            Option<&Camera>,
        ),
        Without<UsdCamera>,
    >,
    mut done: Local<bool>,
    mut wait_ticks: Local<u32>,
    mut last_diag: Local<f32>,
    prims: Query<(), With<UsdPrimRef>>,
    progressive: Option<Res<usd_bevy::ProgressiveProjectionState>>,
) {
    if *done
        || !matches!(*mount, CameraMount::Arcball)
        || extent.count == 0
        || prims.iter().count() == 0
    {
        return;
    }
    // Wait for the extent to stabilize. Bevy populates `Aabb` components
    // for skinned meshes only after the mesh asset is uploaded, which
    // happens a few frames after the prim entities first spawn. Only wait
    // during active projection when the current extent is still degenerate
    // (zero diagonal) so genuine sub-millimetre scenes frame immediately.
    let raw_diag = extent.raw_diag().unwrap_or(0.0);
    if raw_diag <= 0.0
        && progressive.as_ref().is_some_and(|state| {
            matches!(
                state.readiness(),
                usd_bevy::ProjectionReadiness::Planning | usd_bevy::ProjectionReadiness::Projecting
            )
        })
    {
        return;
    }
    *wait_ticks += 1;
    if *wait_ticks < 60 && raw_diag > *last_diag * 1.05 {
        // Extent is still growing — keep waiting.
        *last_diag = raw_diag;
        return;
    }
    let Ok((mut transform, mut cam, pan_orbit, mut projection, camera)) = cameras.single_mut()
    else {
        return;
    };
    let viewport_size = effective_viewport_size(camera);
    let FramingOutcome::Fit(fit) =
        framing_for_scene_extent(&extent, &cam, &projection, viewport_size, &config)
    else {
        return;
    };
    apply_framing_fit(
        fit,
        &mut cam,
        &mut transform,
        pan_orbit.map(Mut::into_inner),
        Some(projection.as_mut()),
        viewport_size,
    );
    *done = true;
    info!(
        "camera framed on scene: focus={:?}, raw_diag={:.6} m, distance={:.6} m, {} prims (waited {} ticks)",
        cam.focus, raw_diag, cam.distance, extent.count, *wait_ticks
    );
    debug!(
        min = ?extent.min,
        max = ?extent.max,
        "world bounds used for initial camera framing"
    );
}

/// Lerp the arcball's focus + distance toward the last-requested
/// target. Zero `remaining` is the sentinel "no tween in flight".
/// Advances the active focus-and-distance tween used by tree navigation.
///
/// Mutually exclusive with mounted authored USD cameras: if `CameraMount` is
/// not `Arcball`, any active tween is cancelled immediately so mounted camera
/// transforms and projections are never overwritten and cancelled tweens
/// cannot resume upon unmount. When a `framing_intent` is attached, it is
/// re-evaluated against the current projection, viewport, and camera basis so
/// mid-transition resizes, FOV changes, or projection switches never finish at
/// a stale pose.
pub(crate) fn apply_fly_to(
    time: Res<Time>,
    mount: Option<Res<CameraMount>>,
    config: Option<Res<CameraNavigationConfig>>,
    mut fly: ResMut<FlyTo>,
    mut cameras: Query<
        (
            &mut Transform,
            &mut ArcballCamera,
            Option<&mut PanOrbitCamera>,
            Option<&mut Projection>,
            Option<&Camera>,
        ),
        Without<UsdCamera>,
    >,
    gate: crate::viewport::navigation::input::ViewerInteractionGate,
) {
    if mount
        .as_deref()
        .is_some_and(|m| !matches!(m, CameraMount::Arcball))
    {
        if fly.remaining > 0.0 || fly.target_ortho_scale.is_some() || fly.framing_intent.is_some() {
            fly.cancel();
        }
        return;
    }
    if fly.remaining <= 0.0 || !fly.remaining.is_finite() {
        fly.cancel();
        return;
    }
    if gate.locked() {
        fly.cancel();
        return;
    }
    if !fly.target_focus.is_finite()
        || !fly.target_distance.is_finite()
        || fly.target_distance <= 0.0
    {
        fly.cancel();
        return;
    }
    let Ok((mut transform, mut cam, pan_orbit, mut projection, camera)) = cameras.single_mut()
    else {
        return;
    };
    let viewport_size = effective_viewport_size(camera);
    let nav_config = config.as_deref().copied().unwrap_or_default();

    // If this transition carries a retained framing intent, re-evaluate it
    // against the live projection, viewport, and orientation before advancing.
    let reevaluated_fit = if let Some(intent) = fly.framing_intent {
        let Some(active_projection) = projection.as_deref() else {
            fly.cancel();
            return;
        };
        let FramingOutcome::Fit(current_fit) =
            frame_intent_for_camera(intent, &cam, active_projection, viewport_size, &nav_config)
        else {
            fly.cancel();
            return;
        };
        fly.target_focus = current_fit.focus;
        fly.target_distance = current_fit.distance;
        match (active_projection, current_fit.orthographic) {
            (Projection::Orthographic(ortho), Some(ortho_fit)) => {
                if fly.start_ortho_scale.is_none() {
                    fly.start_ortho_scale = Some(ortho.scale);
                }
                fly.target_ortho_scale = Some(ortho_fit.scale);
            }
            _ => {
                fly.start_ortho_scale = None;
                fly.target_ortho_scale = None;
            }
        }
        Some(current_fit)
    } else {
        if fly.target_ortho_scale.is_some()
            && !matches!(projection.as_deref(), Some(Projection::Orthographic(_)))
        {
            fly.cancel();
            return;
        }
        None
    };

    let dt = time.delta_secs().min(1.0 / 30.0);
    fly.remaining = (fly.remaining - dt).max(0.0);
    let progress = if fly.duration.is_finite() && fly.duration > 0.0 {
        1.0 - (fly.remaining / fly.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // On the final tick of a framing transition, commit the validated current
    // fit directly via the shared framing applier so all corners/point depth
    // are guaranteed valid in the actual final view.
    if progress >= 1.0
        && let Some(final_fit) = reevaluated_fit
    {
        apply_framing_fit(
            final_fit,
            &mut cam,
            &mut transform,
            pan_orbit.map(Mut::into_inner),
            projection.as_deref_mut(),
            viewport_size,
        );
        fly.cancel();
        return;
    }

    let start_focus = if fly.start_focus.is_finite() {
        fly.start_focus
    } else if cam.focus.is_finite() {
        cam.focus
    } else {
        Vec3::ZERO
    };
    let start_distance = distance_as_f32(sanitize_camera_distance(
        f64::from(fly.start_distance),
        f64::from(cam.distance),
    ));
    let target_distance = distance_as_f32(sanitize_camera_distance(
        f64::from(fly.target_distance),
        f64::from(start_distance),
    ));

    // Cosine ease-out.
    let eased = (progress * core::f32::consts::FRAC_PI_2).sin();

    let next_focus = start_focus.lerp(fly.target_focus, eased);
    let next_distance = start_distance.lerp(target_distance, eased);
    if next_focus.is_finite() {
        cam.focus = next_focus;
    }
    let safe_distance = sanitize_camera_distance(f64::from(next_distance), f64::from(cam.distance));
    cam.distance = distance_as_f32(safe_distance);
    cam.zoom_target = safe_distance;

    // Bookmark restores set start/target yaw + elevation; pick the
    // shortest angular path so a 359° → 1° tween doesn't sweep the
    // long way round.
    if let (Some(sy), Some(ty)) = (fly.start_yaw, fly.target_yaw)
        && sy.is_finite()
        && ty.is_finite()
    {
        cam.yaw = lerp_angle(sy, ty, eased);
    }
    if let (Some(se), Some(te)) = (fly.start_elevation, fly.target_elevation)
        && se.is_finite()
        && te.is_finite()
    {
        cam.elevation = se + (te - se) * eased;
    }
    if let (Some(ss), Some(ts), Some(Projection::Orthographic(ortho))) = (
        fly.start_ortho_scale,
        fly.target_ortho_scale,
        projection.as_deref_mut(),
    ) && ss.is_finite()
        && ss > 0.0
        && ts.is_finite()
        && ts > 0.0
    {
        let next_scale = f64::from(ss) + (f64::from(ts) - f64::from(ss)) * f64::from(eased);
        if let Some((scale_f32, area)) =
            validated_orthographic_scale_area(next_scale, ortho, viewport_size)
        {
            ortho.scale = scale_f32;
            ortho.area = area;
        }
    }
    apply_rig(&cam, &mut transform);
    if let Some(mut pan_orbit) = pan_orbit {
        sync_pan_orbit_from_arcball(&cam, &mut pan_orbit);
    }
    if fly.remaining <= 0.0 {
        fly.cancel();
    }
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

/// Single local clipping authority for the free camera (both perspective and
/// orthographic).
///
/// For perspective cameras, derives `near` strictly from local inspection
/// depth so large scenes never clip close-up detail, and sizes `far` to
/// enclose both local inspection context and valid composed scene bounds.
/// For orthographic cameras, preserves `near = 0.0` and sizes `far` to enclose
/// both local depth and valid scene bounds without repositioning the camera.
/// Mounted authored USD cameras keep their explicit projections untouched.
pub(crate) fn sync_adaptive_camera_clipping(
    mount: Res<CameraMount>,
    config: Res<CameraNavigationConfig>,
    extent: Res<SceneExtent>,
    mut cameras: Query<
        (&Transform, &ArcballCamera, &mut Projection),
        (With<Camera3d>, Without<UsdCamera>),
    >,
) {
    if !matches!(*mount, CameraMount::Arcball) {
        return;
    }

    let scene_bounds = extent.valid_bounds().and_then(|(min, max)| {
        let center = (min + max) * 0.5;
        let radius = f64::from((max - min).length()) * 0.5;
        (center.is_finite() && radius.is_finite()).then_some((center, radius))
    });

    for (transform, cam, mut projection) in cameras.iter_mut() {
        let local_distance =
            sanitize_camera_distance(f64::from(cam.distance), config.empty_scene_distance);
        let (camera_to_scene, scene_radius) = match scene_bounds {
            Some((center, radius)) if transform.translation.is_finite() => {
                (distance_between(transform.translation, center), radius)
            }
            _ => (local_distance, config.empty_scene_radius),
        };
        match projection.as_mut() {
            Projection::Perspective(perspective) => {
                let near = adaptive_near(local_distance, &config);
                let far =
                    adaptive_far(local_distance, camera_to_scene, scene_radius, near, &config);
                if near.is_finite() && far.is_finite() && near > 0.0 && far > near {
                    perspective.near = near;
                    perspective.far = far;
                }
            }
            Projection::Orthographic(ortho) => {
                let near = 0.0_f32;
                let far =
                    adaptive_far(local_distance, camera_to_scene, scene_radius, near, &config);
                if far.is_finite() && far > near {
                    ortho.near = near;
                    ortho.far = far;
                }
            }
            Projection::Custom(_) => {}
        }
    }
}

fn distance_between(a: Vec3, b: Vec3) -> f64 {
    if !a.is_finite() || !b.is_finite() {
        return 0.0;
    }
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    let dz = f64::from(a.z) - f64::from(b.z);
    let distance = dx.hypot(dy).hypot(dz);
    if distance.is_finite() {
        distance
    } else {
        MAX_CAMERA_DISTANCE
    }
}

pub(super) fn adaptive_near(local_distance: f64, config: &CameraNavigationConfig) -> f32 {
    let distance = sanitize_camera_distance(local_distance, config.empty_scene_distance);
    let ratio =
        if config.near_focus_distance_ratio.is_finite() && config.near_focus_distance_ratio > 0.0 {
            config.near_focus_distance_ratio.min(0.25)
        } else {
            1.0e-3
        };
    let max_near = (distance * 0.5).max(MIN_NEAR_PLANE);
    let near = (distance * ratio).clamp(MIN_NEAR_PLANE, max_near);
    let near_f32 = near as f32;
    if near_f32.is_finite() && near_f32 > 0.0 {
        near_f32
    } else {
        MIN_NEAR_PLANE as f32
    }
}

pub(super) fn adaptive_far(
    local_distance: f64,
    camera_to_scene_distance: f64,
    scene_radius: f64,
    near: f32,
    config: &CameraNavigationConfig,
) -> f32 {
    let near_f64 = finite_positive_distance(f64::from(near), MIN_NEAR_PLANE);
    let local_depth = sanitize_camera_distance(local_distance, config.empty_scene_distance);
    let scene_dist = if camera_to_scene_distance.is_finite() && camera_to_scene_distance >= 0.0 {
        camera_to_scene_distance
    } else {
        local_depth
    };
    let radius = if scene_radius.is_finite() && scene_radius >= 0.0 {
        scene_radius
    } else {
        finite_positive_distance(config.empty_scene_radius, 1.0)
    };
    let safety_ratio = if config.far_safety_ratio.is_finite() && config.far_safety_ratio >= 0.0 {
        config.far_safety_ratio
    } else {
        0.1
    };
    let local_far = local_depth * FAR_LOCAL_DEPTH_MULTIPLIER;
    let scene_far = scene_dist + radius + (radius * safety_ratio).max(near_f64);
    let far_f64 = local_far.max(scene_far).max(near_f64 * 16.0);
    let clamped = if far_f64.is_finite() {
        far_f64.clamp(MIN_CAMERA_DISTANCE, f64::from(f32::MAX) * 0.5)
    } else {
        f64::from(f32::MAX) * 0.5
    };
    let far = clamped as f32;
    if far.is_finite() && far > near {
        far
    } else {
        (near * 16.0).max(1.0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/camera/navigation_inline_tests.rs"]
mod tests;
