//! Orbit/pan/zoom camera controller backed by Bevy's `PanOrbitCamera`.
//!
//! - Scroll            → focus-centered exponential zoom
//! - Middle/Shift drag → pixel-locked screen-space pan
//! - Left + Right drag → orbit around focus (fixed +Y up)
//!
//! Run conditions yield to host UI when a panel wants the pointer, so orbiting
//! doesn't fight with panel scroll / sliders.

mod framing;
mod glacial;
mod navigation;
mod state;
#[cfg(test)]
#[path = "../../../../tests/unit/viewport/camera/zoom_tests.rs"]
mod zoom_tests;

use bevy::camera::Projection;
use bevy::camera_controller::pan_orbit_camera::{
    SyncCameraPosition,
    controller::{
        component::{EnabledMotion, OrbitConstraint, PanOrbitCamera, Sensitivity},
        momentum::Momentum,
        motion::CurrentMotion,
        smoothing::Smoothing,
        zoom::ZoomLimits,
    },
};
use bevy::math::DVec3;
use bevy::prelude::*;
use usd_bevy::UsdCamera;
use viewport_protocol::{CameraOrientationReadModel, ViewportEvent, ViewportEventEnvelope};

use crate::viewport::navigation::input::{
    ViewportNavigationInput, apply_local_navigation_input, reset_navigation_frame,
    ui_wants_pointer_input,
};
use crate::viewport::presentation::ViewportEventOutbox;

pub(crate) use framing::{
    FramingFit, FramingOutcome, apply_framing_fit, effective_viewport_size,
    exponential_orthographic_scale, frame_bounds_for_camera, frame_intent_for_camera,
    frame_point_for_camera, framing_for_scene_extent, orthographic_scale_bounds,
    validated_orthographic_scale_area,
};
pub(crate) use glacial::sync_chase_camera;
pub(crate) use navigation::{
    apply_fly_to, fit_camera_once, follow_mounted_camera, sync_adaptive_camera_clipping,
};
pub(crate) use state::{
    CameraBookmark, CameraBookmarks, CameraMount, CameraNavigationConfig, CameraOrientationState,
    FlyTo, FramingIntent,
};

/// Lower endpoint of the chosen CAD/BIM engineering camera-to-focus distance
/// envelope (`0.1` micrometres).
///
/// This is an engineering navigation envelope rather than a universal `f32`
/// representability bound: zooming inward saturates cleanly at this endpoint
/// (discarding excess inward scroll) and reverses immediately on outward input.
/// At large world-space focus offsets, `f32` `Transform::translation` ULP
/// quantization can still limit representable sub-step motion.
pub(crate) const MIN_CAMERA_DISTANCE: f64 = 1.0e-7;

/// Upper endpoint of the chosen CAD/BIM engineering camera-to-focus distance
/// envelope (`1e12` metres).
///
/// This is an engineering navigation envelope rather than a universal `f32`
/// representability bound: zooming outward saturates cleanly at this endpoint
/// and reverses immediately on inward input.
pub(crate) const MAX_CAMERA_DISTANCE: f64 = 1.0e12;

pub struct ArcballCameraPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArcballCameraSet {
    PrepareInput,
    ApplyInput,
}

impl Plugin for ArcballCameraPlugin {
    fn build(&self, app: &mut App) {
        if !app
            .world()
            .contains_resource::<Messages<bevy::window::RequestRedraw>>()
        {
            app.add_message::<bevy::window::RequestRedraw>();
        }
        // Register ONLY native PanOrbitCamera position updates without Bevy's
        // `update_perspective` or `update_orthographic` projection writers so
        // `sync_adaptive_camera_clipping` is the single clipping authority for
        // free perspective and orthographic cameras, orthographic framing depth
        // is never overwritten on subsequent ticks, and mounted USD cameras
        // keep their authored projections untouched.
        app.init_resource::<CameraMount>()
            .init_resource::<crate::viewport::navigation::input::HostUiInputCapture>()
            .init_resource::<ViewportNavigationInput>()
            .init_resource::<CameraNavigationConfig>()
            .init_resource::<CameraOrientationState>()
            .configure_sets(
                Update,
                (ArcballCameraSet::PrepareInput, ArcballCameraSet::ApplyInput).chain(),
            )
            .add_systems(
                PreUpdate,
                PanOrbitCamera::update_camera_positions
                    .run_if(arcball_is_active)
                    .in_set(SyncCameraPosition),
            )
            .add_systems(
                PreUpdate,
                sync_arcball_from_pan_orbit
                    .run_if(arcball_is_active)
                    .after(SyncCameraPosition),
            )
            .add_systems(
                Update,
                (reset_navigation_frame, apply_local_navigation_input)
                    .chain()
                    .in_set(ArcballCameraSet::PrepareInput),
            )
            .add_systems(
                Update,
                drive_pan_orbit_camera
                    .run_if(arcball_is_active)
                    .in_set(ArcballCameraSet::ApplyInput),
            )
            .add_systems(
                Update,
                sync_adaptive_camera_clipping
                    .run_if(arcball_is_active)
                    .after(crate::viewport::rendering::scene::extent::compute_extent)
                    .after(ArcballCameraSet::ApplyInput),
            )
            .add_systems(PostUpdate, publish_camera_orientation);
    }
}

/// Enables free-camera input only while no authored USD camera is mounted.
fn arcball_is_active(mount: Res<CameraMount>) -> bool {
    matches!(*mount, CameraMount::Arcball)
}

/// Product camera rig state synchronized with Bevy's `PanOrbitCamera` motion authority.
#[derive(Component, Clone)]
pub struct ArcballCamera {
    pub focus: Vec3,
    pub yaw: f32,
    pub elevation: f32,
    pub distance: f32,
    pub zoom_target: f64,
    pub pan_sensitivity: f32,
    pub orbit_speed: f32,
    pub zoom_smoothing: f64,
}

impl Default for ArcballCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.0,
            elevation: 25f32.to_radians(),
            distance: 4.0,
            zoom_target: 4.0,
            pan_sensitivity: 1.15,
            orbit_speed: 0.005,
            zoom_smoothing: 18.0,
        }
    }
}

/// Constructs a `PanOrbitCamera` configured for CAD/BIM extreme-scale
/// navigation with fixed `+Y` up. Native `PanOrbitCamera` handles pan and
/// orbit while focus-centered exponential dolly owns wheel zoom.
pub(crate) fn default_pan_orbit_camera(distance: f32) -> PanOrbitCamera {
    let safe_distance = sanitize_camera_distance(f64::from(distance), 1.0);
    PanOrbitCamera {
        orbit_constraint: OrbitConstraint::Fixed {
            up: DVec3::Y,
            can_pass_tdc: false,
        },
        enabled_motion: EnabledMotion {
            pan: true,
            orbit: true,
            zoom: false,
        },
        zoom_limits: ZoomLimits {
            min_size_per_pixel: 1e-12,
            max_size_per_pixel: 1e27,
            zoom_through_objects: true,
        },
        smoothing: Smoothing {
            orbit: std::time::Duration::ZERO,
            pan: std::time::Duration::ZERO,
            zoom: std::time::Duration::ZERO,
        },
        momentum: Momentum {
            orbit_damping: 255,
            orbit_friction: 100.0,
            init_orbit: std::time::Duration::ZERO,
            pan_damping: 255,
            pan_friction: 100.0,
            init_pan: std::time::Duration::ZERO,
        },
        input_debounce: std::time::Duration::ZERO,
        sensitivity: Sensitivity {
            orbit: Vec2::splat(1.0),
            zoom: 1.0,
        },
        last_anchor_depth: -safe_distance,
        ..Default::default()
    }
}

/// Synchronizes `PanOrbitCamera`'s view-space anchor depth after programmatic
/// `ArcballCamera` changes (`fit_camera_once`, `FlyTo`, `follow_mounted_camera`).
pub(crate) fn sync_pan_orbit_from_arcball(cam: &ArcballCamera, pan_orbit: &mut PanOrbitCamera) {
    pan_orbit.last_anchor_depth = -sanitize_camera_distance(f64::from(cam.distance), 1.0);
    pan_orbit.current_motion = CurrentMotion::Stationary;
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ActiveMotion {
    #[default]
    None,
    Pan,
    Orbit,
}

/// Routes unified native and remote `ViewportNavigationInput` into native
/// `PanOrbitCamera` pan/orbit and projection-appropriate exponential zoom
/// (camera-to-focus dolly for perspective; decoupled log-scale magnification
/// for orthographic without gratuitous depth translation).
fn drive_pan_orbit_camera(
    input: Res<ViewportNavigationInput>,
    capture: Option<Res<crate::viewport::navigation::input::HostUiInputCapture>>,
    gate: crate::viewport::navigation::input::ViewerInteractionGate,
    config: Res<CameraNavigationConfig>,
    mut fly_to: Option<ResMut<FlyTo>>,
    mut active: Local<ActiveMotion>,
    mut cameras: Query<(
        &mut Transform,
        &mut ArcballCamera,
        &mut PanOrbitCamera,
        Option<&mut Projection>,
        Option<&Camera>,
    )>,
) {
    if gate.locked() || !input.focused || ui_wants_pointer_input(capture) {
        if *active != ActiveMotion::None {
            for (_, _, mut pan_orbit, _, _) in cameras.iter_mut() {
                pan_orbit.end_move();
            }
            *active = ActiveMotion::None;
        }
        return;
    }

    let pan_drag = !input.modifiers.control
        && (input.buttons.auxiliary
            || (input.modifiers.shift && (input.buttons.primary || input.buttons.secondary)));
    let both_lr = !input.modifiers.shift
        && !input.modifiers.control
        && input.buttons.primary
        && input.buttons.secondary;
    let pan_multiplier = if input.pan_multiplier.is_finite() && input.pan_multiplier > 0.0 {
        input.pan_multiplier
    } else {
        1.0
    };
    let scroll_delta = if input.wheel_delta.y.is_finite() {
        input.wheel_delta.y
    } else {
        0.0
    };

    // Active user navigation supersedes any in-flight programmatic FlyTo tween
    // so manual gestures and focus transitions never compete.
    if (pan_drag || both_lr || scroll_delta != 0.0)
        && let Some(fly) = fly_to.as_deref_mut()
        && (fly.remaining > 0.0 || fly.framing_intent.is_some())
    {
        fly.cancel();
    }

    for (mut transform, mut cam, mut pan_orbit, mut projection, camera) in cameras.iter_mut() {
        if !cam.focus.is_finite() {
            cam.focus = Vec3::ZERO;
        }
        let current_distance =
            sanitize_camera_distance(f64::from(cam.distance), config.empty_scene_distance);
        cam.distance = distance_as_f32(current_distance);
        cam.zoom_target = current_distance;

        // Recover corrupted or inverted PanOrbitCamera anchor depths before
        // any zoom, pan, or orbit motion executes.
        if !pan_orbit.last_anchor_depth.is_finite()
            || pan_orbit.last_anchor_depth >= 0.0
            || pan_orbit.last_anchor_depth.abs() < MIN_CAMERA_DISTANCE
        {
            pan_orbit.last_anchor_depth = -current_distance;
        }

        let mut active_distance = current_distance;
        if scroll_delta != 0.0 {
            match projection.as_deref_mut() {
                Some(Projection::Orthographic(ortho)) => {
                    // Orthographic magnification operates strictly on scale/area
                    // in log domain, decoupled from camera-distance limits and
                    // without translating the camera in depth.
                    let viewport_size = effective_viewport_size(camera);
                    if let Some((min_scale, max_scale)) =
                        orthographic_scale_bounds(ortho, viewport_size)
                        && let Some(next_scale) = exponential_orthographic_scale(
                            f64::from(ortho.scale),
                            f64::from(scroll_delta),
                            config.zoom_speed,
                            min_scale,
                            max_scale,
                        )
                        && let Some((next_scale_f32, next_area)) =
                            validated_orthographic_scale_area(next_scale, ortho, viewport_size)
                    {
                        ortho.scale = next_scale_f32;
                        ortho.area = next_area;
                    }
                }
                Some(Projection::Perspective(_)) | None => {
                    let next_distance = exponential_dolly_distance(
                        current_distance,
                        f64::from(scroll_delta),
                        config.zoom_speed,
                    );
                    cam.distance = distance_as_f32(next_distance);
                    cam.zoom_target = next_distance;
                    apply_rig(&cam, &mut transform);
                    pan_orbit.last_anchor_depth = -next_distance;
                    if let CurrentMotion::UserControlled { ref mut anchor, .. } =
                        pan_orbit.current_motion
                    {
                        anchor.z = -next_distance;
                    }
                    active_distance = next_distance;
                }
                Some(Projection::Custom(_)) => {}
            }
        }

        // Focus-centered anchor in camera view space keeps pan and orbit
        // centered on `cam.focus`.
        let focus_anchor = DVec3::new(0.0, 0.0, -active_distance);

        if pan_drag {
            if *active != ActiveMotion::Pan {
                pan_orbit.start_pan(Some(focus_anchor));
                *active = ActiveMotion::Pan;
            }
            if let Some(final_pan) =
                validated_pan_delta(input.pointer_delta, pan_multiplier, cam.pan_sensitivity)
            {
                pan_orbit.send_screenspace_input(final_pan);
            }
        } else if both_lr {
            if *active != ActiveMotion::Orbit {
                pan_orbit.start_orbit(Some(focus_anchor));
                *active = ActiveMotion::Orbit;
            }
            if let Some(final_orbit) = validated_orbit_delta(input.pointer_delta) {
                pan_orbit.send_screenspace_input(final_orbit);
            }
        } else if *active != ActiveMotion::None {
            pan_orbit.end_move();
            *active = ActiveMotion::None;
        }
    }
}

/// Validates the final scaled pan vector after applying both transport
/// multiplier and camera sensitivity so non-finite overflow never reaches
/// `PanOrbitCamera`.
fn validated_pan_delta(raw_delta: Vec2, pan_multiplier: f32, pan_sensitivity: f32) -> Option<Vec2> {
    if !raw_delta.is_finite()
        || !pan_multiplier.is_finite()
        || pan_multiplier <= 0.0
        || !pan_sensitivity.is_finite()
        || pan_sensitivity <= 0.0
    {
        return None;
    }
    let scale = f64::from(pan_multiplier) * f64::from(pan_sensitivity);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let candidate = Vec2::new(
        (f64::from(raw_delta.x) * scale) as f32,
        (f64::from(raw_delta.y) * scale) as f32,
    );
    (candidate.is_finite() && candidate != Vec2::ZERO).then_some(candidate)
}

/// Validates the final orbit screen-space vector before forwarding it to
/// `PanOrbitCamera`.
fn validated_orbit_delta(raw_delta: Vec2) -> Option<Vec2> {
    (raw_delta.is_finite() && raw_delta != Vec2::ZERO).then_some(raw_delta)
}

/// Mirrors `PanOrbitCamera` transform/anchor updates back into `ArcballCamera`
/// so bookmarks, `FlyTo`, `ChaseCamera`, and local clipping read live state.
fn sync_arcball_from_pan_orbit(
    mount: Res<CameraMount>,
    mut cameras: Query<(&Transform, &PanOrbitCamera, &mut ArcballCamera)>,
) {
    if !matches!(*mount, CameraMount::Arcball) {
        return;
    }
    for (transform, pan_orbit, mut cam) in cameras.iter_mut() {
        if matches!(pan_orbit.current_motion, CurrentMotion::Stationary) {
            continue;
        }
        if !transform.translation.is_finite() || !transform.rotation.is_finite() {
            continue;
        }
        let anchor_depth = -pan_orbit.last_anchor_depth();
        let safe_distance = sanitize_camera_distance(anchor_depth, f64::from(cam.distance));
        let distance = distance_as_f32(safe_distance);
        let forward = transform.rotation * Vec3::NEG_Z;
        if !forward.is_finite() {
            continue;
        }
        let focus = transform.translation + forward * distance;
        if focus.is_finite() {
            cam.focus = focus;
        }
        cam.distance = distance;
        cam.zoom_target = safe_distance;
        let offset = -forward;
        let horiz = offset.x.hypot(offset.z);
        cam.elevation = offset
            .y
            .clamp(-1.0, 1.0)
            .asin()
            .clamp((-89f32).to_radians(), 89f32.to_radians());
        if horiz > 1.0e-5 {
            cam.yaw = offset.x.atan2(offset.z);
        }
    }
}

pub(crate) fn sanitize_camera_distance(value: f64, fallback: f64) -> f64 {
    let safe_fallback = if fallback.is_finite() && fallback > 0.0 {
        fallback.clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE)
    } else {
        1.0
    };
    if value.is_finite() && value > 0.0 {
        value.clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE)
    } else {
        safe_fallback
    }
}

fn finite_positive_distance(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else if fallback.is_finite() && fallback > 0.0 {
        fallback
    } else {
        1.0
    }
}

fn distance_as_f32(value: f64) -> f32 {
    let candidate = sanitize_camera_distance(value, 1.0);
    let distance = candidate as f32;
    if distance.is_finite() && distance > 0.0 {
        distance
    } else if candidate >= MAX_CAMERA_DISTANCE {
        MAX_CAMERA_DISTANCE as f32
    } else {
        MIN_CAMERA_DISTANCE as f32
    }
}

/// Computes the next camera-to-focus distance for a scroll delta using
/// frame-partition-invariant exponential dolly saturated only against the
/// engineering envelope `[MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE]`.
///
/// Excess same-direction input beyond either endpoint saturates cleanly at the
/// bound, and opposite-direction input reverses away from the bound immediately.
pub(crate) fn exponential_dolly_distance(current: f64, scroll_delta: f64, zoom_speed: f64) -> f64 {
    let base = sanitize_camera_distance(current, 1.0);
    if !scroll_delta.is_finite()
        || scroll_delta == 0.0
        || !zoom_speed.is_finite()
        || zoom_speed <= 0.0
    {
        return base;
    }
    let lo = MIN_CAMERA_DISTANCE.ln();
    let hi = MAX_CAMERA_DISTANCE.ln();
    let b = base.ln().clamp(lo, hi);
    let target = if scroll_delta > 0.0 {
        let room = (b - lo).max(0.0);
        let step = if room == 0.0 || scroll_delta >= room / zoom_speed {
            room
        } else {
            scroll_delta * zoom_speed
        };
        b - step
    } else {
        let room = (hi - b).max(0.0);
        let magnitude = -scroll_delta;
        let step = if room == 0.0 || magnitude >= room / zoom_speed {
            room
        } else {
            magnitude * zoom_speed
        };
        b + step
    };
    let clamped = target.clamp(lo, hi);
    if !clamped.is_finite() {
        return base;
    }
    let next = clamped.exp();
    if next.is_finite() && next > 0.0 {
        next.clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE)
    } else {
        base
    }
}

/// Rebuilds the camera transform from its focus, yaw, elevation, and distance.
pub(crate) fn apply_rig(cam: &ArcballCamera, tr: &mut Transform) {
    let focus = if cam.focus.is_finite() {
        cam.focus
    } else {
        Vec3::ZERO
    };
    let distance = distance_as_f32(f64::from(cam.distance));
    let yaw = if cam.yaw.is_finite() { cam.yaw } else { 0.0 };
    let elevation = if cam.elevation.is_finite() {
        cam.elevation
    } else {
        0.0
    };
    let cos_e = elevation.cos();
    let sin_e = elevation.sin();
    let dir_from_focus = Vec3::new(cos_e * yaw.sin(), sin_e, cos_e * yaw.cos());
    let cam_world = focus + dir_from_focus * distance;
    let up = if cos_e.abs() <= 1e-5 {
        // At the poles the world-Y up vector is parallel to the view
        // direction. Preserve the canonical CAD roll instead of sharing one
        // fallback: Top looks down with -Z up, Bottom looks up with +Z up.
        if sin_e >= 0.0 { Vec3::NEG_Z } else { Vec3::Z }
    } else {
        Vec3::Y
    };
    *tr = Transform::from_translation(cam_world).looking_to(-dir_from_focus, up);
}

fn publish_camera_orientation(
    time: Res<Time>,
    mut state: ResMut<CameraOrientationState>,
    outbox: Option<ResMut<ViewportEventOutbox>>,
    cameras: Query<&Transform, (With<Camera3d>, Without<UsdCamera>)>,
) {
    let Some(mut outbox) = outbox else {
        return;
    };
    let Ok(transform) = cameras.single() else {
        return;
    };
    let rotation = transform.rotation;
    let raw = rotation.to_array();
    if !raw.iter().all(|value| value.is_finite()) {
        return;
    }
    if state.last_rotation.is_some_and(|last| {
        last.to_array()
            .iter()
            .zip(raw.iter())
            .all(|(left, right)| (left - right).abs() <= 1e-5)
    }) {
        return;
    }

    let now = time.elapsed_secs_f64();
    if state.has_published && now - state.last_published_at < 1.0 / 30.0 {
        return;
    }
    let Some(orientation) = CameraOrientationReadModel::from_rotation_xyzw(raw) else {
        return;
    };
    state.last_rotation = Some(rotation);
    state.last_published_at = now;
    state.has_published = true;
    state.latest = orientation;
    outbox.push(ViewportEventEnvelope::new(
        None,
        ViewportEvent::CameraOrientationChanged { orientation },
    ));
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/camera/mod_inline_tests.rs"]
mod tests;
