//! Orbit/pan/zoom camera controller backed by Bevy's `PanOrbitCamera`.
//!
//! - Scroll            → focus-centered exponential zoom
//! - Middle/Shift drag → pixel-locked screen-space pan
//! - Left + Right drag → orbit around focus (fixed +Y up)
//!
//! Run conditions yield to host UI when a panel wants the pointer, so orbiting
//! doesn't fight with panel scroll / sliders.

mod glacial;
mod navigation;
mod state;
#[cfg(test)]
#[path = "../../../tests/unit/viewport/camera/zoom_tests.rs"]
mod zoom_tests;

use bevy::camera::Projection;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::camera_controller::pan_orbit_camera::{
    SyncCameraPosition,
    controller::{
        MinimalPanOrbitCameraPlugin,
        component::{OrbitConstraint, PanOrbitCamera, Sensitivity},
        momentum::Momentum,
        motion::CurrentMotion,
        smoothing::Smoothing,
        zoom::ZoomLimits,
    },
};
use usd_bevy::UsdCamera;
use viewport_protocol::{CameraOrientationReadModel, ViewportEvent, ViewportEventEnvelope};

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::input::{
    ViewportNavigationInput, apply_local_navigation_input, reset_navigation_frame,
    ui_wants_pointer_input,
};

pub(crate) use glacial::sync_chase_camera;
pub(crate) use navigation::{
    apply_fly_to, fit_camera_once, follow_mounted_camera, sync_adaptive_camera_clipping,
};
pub(crate) use state::{
    CameraBookmark, CameraBookmarks, CameraMount, CameraNavigationConfig, CameraOrientationState,
    FlyTo,
};

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
        if !app.is_plugin_added::<MinimalPanOrbitCameraPlugin>() {
            app.add_plugins(MinimalPanOrbitCameraPlugin);
        }
        // Camera controller yields to host UI (so scrolling a panel doesn't zoom)
        // AND to the Cameras tab (so a mounted USD camera isn't fought by
        // orbit/pan input).
        app.init_resource::<crate::viewport::input::HostUiInputCapture>()
            .init_resource::<ViewportNavigationInput>()
            .init_resource::<CameraNavigationConfig>()
            .init_resource::<CameraOrientationState>()
            .configure_sets(
                Update,
                (ArcballCameraSet::PrepareInput, ArcballCameraSet::ApplyInput).chain(),
            )
            .add_systems(
                PreUpdate,
                sync_arcball_from_pan_orbit.after(SyncCameraPosition),
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
                    .after(crate::viewport::scene::extent::compute_extent)
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
/// navigation with fixed `+Y` up and no cursor-zoom drift.
pub(crate) fn default_pan_orbit_camera(distance: f32) -> PanOrbitCamera {
    PanOrbitCamera {
        orbit_constraint: OrbitConstraint::Fixed {
            up: DVec3::Y,
            can_pass_tdc: false,
        },
        zoom_limits: ZoomLimits {
            min_size_per_pixel: 1e-6,
            max_size_per_pixel: 1e27,
            zoom_through_objects: false,
        },
        smoothing: Smoothing {
            orbit: std::time::Duration::ZERO,
            pan: std::time::Duration::ZERO,
            zoom: std::time::Duration::from_millis(25),
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
        last_anchor_depth: -(f64::from(distance.max(1.0e-5))),
        ..Default::default()
    }
}

/// Synchronizes `PanOrbitCamera`'s view-space anchor depth after programmatic
/// `ArcballCamera` changes (`fit_camera_once`, `FlyTo`, `follow_mounted_camera`).
pub(crate) fn sync_pan_orbit_from_arcball(
    cam: &ArcballCamera,
    pan_orbit: &mut PanOrbitCamera,
) {
    pan_orbit.last_anchor_depth = -(f64::from(cam.distance.max(1.0e-5)));
    pan_orbit.current_motion = CurrentMotion::Stationary;
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ActiveMotion {
    #[default]
    None,
    Pan,
    Orbit,
    Zoom,
}

/// Routes unified native and remote `ViewportNavigationInput` into Bevy's
/// single `PanOrbitCamera` motion authority using focus-centered anchors.
fn drive_pan_orbit_camera(
    input: Res<ViewportNavigationInput>,
    capture: Option<Res<crate::viewport::input::HostUiInputCapture>>,
    config: Res<CameraNavigationConfig>,
    mut active: Local<ActiveMotion>,
    mut cameras: Query<(&ArcballCamera, &mut PanOrbitCamera)>,
) {
    if !input.focused || ui_wants_pointer_input(capture) {
        if *active != ActiveMotion::None {
            for (_, mut pan_orbit) in cameras.iter_mut() {
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
    let pan_delta = if pan_drag {
        input.pointer_delta * input.pan_multiplier
    } else {
        Vec2::ZERO
    };
    let orbit_delta = if both_lr {
        input.pointer_delta
    } else {
        Vec2::ZERO
    };
    let scroll_delta = input.wheel_delta.y;
    let viewport_height = input.viewport_size.y.max(1.0) as f64;

    for (cam, mut pan_orbit) in cameras.iter_mut() {
        // Focus-centered anchor in camera view space keeps scroll centered on
        // `cam.focus` instead of drifting toward the cursor.
        let focus_anchor = DVec3::new(0.0, 0.0, -(f64::from(cam.distance.max(1.0e-5))));

        if pan_drag {
            if *active != ActiveMotion::Pan {
                pan_orbit.start_pan(Some(focus_anchor));
                *active = ActiveMotion::Pan;
            }
            if pan_delta != Vec2::ZERO {
                pan_orbit.send_screenspace_input(pan_delta * cam.pan_sensitivity);
            }
            if scroll_delta != 0.0 {
                let zoom_input = (f64::from(scroll_delta) * config.zoom_speed * viewport_height) as f32;
                pan_orbit.send_zoom_input(zoom_input);
            }
        } else if both_lr {
            if *active != ActiveMotion::Orbit {
                pan_orbit.start_orbit(Some(focus_anchor));
                *active = ActiveMotion::Orbit;
            }
            if orbit_delta != Vec2::ZERO {
                pan_orbit.send_screenspace_input(orbit_delta);
            }
            if scroll_delta != 0.0 {
                let zoom_input = (f64::from(scroll_delta) * config.zoom_speed * viewport_height) as f32;
                pan_orbit.send_zoom_input(zoom_input);
            }
        } else if scroll_delta != 0.0 {
            if *active != ActiveMotion::Zoom {
                pan_orbit.start_zoom(Some(focus_anchor));
                *active = ActiveMotion::Zoom;
            }
            let zoom_input = (f64::from(scroll_delta) * config.zoom_speed * viewport_height) as f32;
            pan_orbit.send_zoom_input(zoom_input);
        } else if *active != ActiveMotion::None {
            pan_orbit.end_move();
            *active = ActiveMotion::None;
        }
    }
}

/// Mirrors `PanOrbitCamera` transform/anchor updates back into `ArcballCamera`
/// so bookmarks, `FlyTo`, `ChaseCamera`, and extent clipping read live state.
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
        let distance = distance_as_f32(-pan_orbit.last_anchor_depth());
        let forward = transform.rotation * Vec3::NEG_Z;
        let focus = transform.translation + forward * distance;
        if focus.is_finite() {
            cam.focus = focus;
        }
        cam.distance = distance;
        cam.zoom_target = f64::from(distance);
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

fn finite_positive_distance(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else if value.is_sign_positive() {
        f64::MAX
    } else {
        fallback.max(f64::from(f32::MIN_POSITIVE))
    }
}

fn distance_as_f32(value: f64) -> f32 {
    let candidate = if value.is_finite() && value > 0.0 {
        value
    } else if value.is_sign_positive() {
        f64::from(f32::MAX)
    } else {
        f64::from(f32::MIN_POSITIVE)
    };
    let distance = candidate as f32;
    if distance.is_finite() && distance > 0.0 {
        distance
    } else if candidate.is_sign_positive() {
        f32::MAX
    } else {
        f32::MIN_POSITIVE
    }
}

#[allow(dead_code)]
fn zoom_target_after_scroll(target: f64, scroll_delta: f64, zoom_speed: f64) -> f64 {
    let current = finite_positive_distance(target, 1.0);
    let exponent = -scroll_delta * zoom_speed;
    let next_log = current.ln() + exponent;
    if next_log.is_nan() {
        return current;
    }
    if next_log > f64::MAX.ln() {
        f64::MAX
    } else if next_log < f64::MIN_POSITIVE.ln() {
        f64::MIN_POSITIVE
    } else {
        next_log.exp()
    }
}

/// Rebuilds the camera transform from its focus, yaw, elevation, and distance.
pub(crate) fn apply_rig(cam: &ArcballCamera, tr: &mut Transform) {
    let horizontal = cam.distance * cam.elevation.cos();
    let vertical = cam.distance * cam.elevation.sin();
    let offset = Vec3::new(
        horizontal * cam.yaw.sin(),
        vertical,
        horizontal * cam.yaw.cos(),
    );
    let cam_world = cam.focus + offset;
    let up = if horizontal.abs() <= 1e-5 {
        // At the poles the world-Y up vector is parallel to the view
        // direction. Preserve the canonical CAD roll instead of sharing one
        // fallback: Top looks down with -Z up, Bottom looks up with +Z up.
        if vertical >= 0.0 {
            Vec3::NEG_Z
        } else {
            Vec3::Z
        }
    } else {
        Vec3::Y
    };
    *tr = Transform::from_translation(cam_world).looking_at(cam.focus, up);
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
#[path = "../../../tests/unit/viewport/camera/mod_inline_tests.rs"]
mod tests;
