//! Orbit camera — ported verbatim from
//! `../bevy_urdf/src/camera.rs`. Astrocraft-style rig:
//!
//! - Scroll            → zoom (logarithmic, smoothed, close-inspection friendly)
//! - Middle/Shift drag → pan (screen-space, not ground-plane locked)
//! - Left + Right drag → orbit (yaw + elevation, clamped -89°–89°)
//!
//! Run conditions yield to egui when a panel wants the pointer, so orbiting
//! doesn't fight with panel scroll / sliders.

mod glacial;
mod navigation;
mod state;

use bevy::camera::Projection;
use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_pointer_input;
use usd_bevy::UsdCamera;
use viewport_protocol::{CameraOrientationReadModel, ViewportEvent, ViewportEventEnvelope};

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::input::{
    ViewportNavigationInput, apply_local_navigation_input, reset_navigation_frame,
};

pub(crate) use glacial::sync_chase_camera;
pub(crate) use navigation::{apply_fly_to, fit_camera_once, follow_mounted_camera};
pub(crate) use state::{
    CameraBookmark, CameraBookmarks, CameraMount, CameraOrientationState, FlyTo,
};

pub struct ArcballCameraPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArcballCameraSet {
    PrepareInput,
    ApplyInput,
}

impl Plugin for ArcballCameraPlugin {
    fn build(&self, app: &mut App) {
        // Arcball yields to egui (so scrolling a panel doesn't zoom) AND
        // to the Cameras tab (so a mounted USD camera isn't fought by
        // orbit/pan input). Tiny run-condition saves a lot of confusion.
        app.init_resource::<ViewportNavigationInput>()
            .init_resource::<CameraOrientationState>()
            .configure_sets(
                Update,
                (ArcballCameraSet::PrepareInput, ArcballCameraSet::ApplyInput).chain(),
            )
            .add_systems(
                Update,
                (reset_navigation_frame, apply_local_navigation_input)
                    .chain()
                    .in_set(ArcballCameraSet::PrepareInput),
            )
            .add_systems(
                Update,
                (drive_arcball, drive_arcball_zoom)
                    .run_if(not(egui_wants_any_pointer_input))
                    .run_if(arcball_is_active)
                    .in_set(ArcballCameraSet::ApplyInput),
            )
            .add_systems(PostUpdate, publish_camera_orientation);
    }
}

/// Enables free-camera input only while no authored USD camera is mounted.
fn arcball_is_active(mount: Res<CameraMount>) -> bool {
    matches!(*mount, CameraMount::Arcball)
}

#[derive(Component, Clone)]
pub struct ArcballCamera {
    pub focus: Vec3,
    pub yaw: f32,
    pub elevation: f32,
    pub distance: f32,
    pub zoom_target: f64,
    pub min_distance: f32,
    pub max_distance: f32,
    pub pan_sensitivity: f32,
    pub orbit_speed: f32,
    pub zoom_step: f64,
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
            min_distance: 0.001,
            max_distance: 60.0,
            pan_sensitivity: 1.15,
            orbit_speed: 0.005,
            zoom_step: 0.12,
            zoom_smoothing: 18.0,
        }
    }
}

/// Converts active mouse drags into screen-space panning and orbit updates.
fn drive_arcball(
    input: Res<ViewportNavigationInput>,
    mut cameras: Query<(&mut Transform, &mut ArcballCamera, &Projection)>,
) {
    if !input.focused {
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
    let window_height = input.viewport_size.y.max(1.0);

    for (mut tr, mut cam, projection) in cameras.iter_mut() {
        if pan_delta != Vec2::ZERO {
            let pan_world = screen_space_pan_delta(&tr, &cam, projection, pan_delta, window_height);
            cam.focus += pan_world;
        }
        if orbit_delta != Vec2::ZERO {
            cam.yaw -= orbit_delta.x * cam.orbit_speed;
            cam.elevation += orbit_delta.y * cam.orbit_speed;
            cam.elevation = cam
                .elevation
                .clamp((-89f32).to_radians(), 89f32.to_radians());
        }
        apply_rig(&cam, &mut tr);
    }
}

/// Converts a cursor delta to world space at the camera's current depth.
fn screen_space_pan_delta(
    tr: &Transform,
    cam: &ArcballCamera,
    projection: &Projection,
    pan_delta: Vec2,
    window_height: f32,
) -> Vec3 {
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => core::f32::consts::FRAC_PI_4,
    };
    let world_units_per_pixel =
        2.0 * cam.distance.max(cam.min_distance) * (fov * 0.5).tan() / window_height;
    let right = tr.rotation * Vec3::X;
    let up = tr.rotation * Vec3::Y;
    (-right * pan_delta.x + up * pan_delta.y) * world_units_per_pixel * cam.pan_sensitivity
}

/// Applies logarithmic, smoothed scroll-wheel zoom within configured bounds.
fn drive_arcball_zoom(
    time: Res<Time>,
    input: Res<ViewportNavigationInput>,
    mut cameras: Query<(&mut Transform, &mut ArcballCamera)>,
) {
    let scroll_delta = input.wheel_delta.y as f64;

    for (mut tr, mut cam) in cameras.iter_mut() {
        let min = cam.min_distance as f64;
        let max = cam.max_distance as f64;
        let mut target = cam.zoom_target;

        if scroll_delta != 0.0 {
            let log_target = target.max(0.01).log10();
            let new_log = log_target - scroll_delta * cam.zoom_step;
            target = 10f64.powf(new_log).clamp(min, max);
        } else if target < min || target > max {
            target = target.clamp(min, max);
        }

        cam.zoom_target = target;

        let dt = time.delta_secs_f64();
        let log_current = (cam.distance as f64).max(0.01).ln();
        let log_target = target.max(0.01).ln();
        let log_diff = log_target - log_current;
        if log_diff.abs() > 1e-4 {
            let new_log = log_current + log_diff * (cam.zoom_smoothing * dt).min(0.9);
            cam.distance = new_log.exp() as f32;
            apply_rig(&cam, &mut tr);
        } else if log_diff.abs() > 1e-5 {
            cam.distance = target as f32;
            apply_rig(&cam, &mut tr);
        }
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
        // At the Top/Bottom poles the world-Y up vector is parallel to the
        // view direction and produces an unstable basis. World-Z is a stable
        // fallback for those two canonical views.
        Vec3::Z
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
mod tests {
    use super::*;

    #[test]
    fn pole_rig_uses_a_stable_up_axis() {
        let cam = ArcballCamera {
            elevation: core::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let mut transform = Transform::default();
        apply_rig(&cam, &mut transform);
        assert!(transform.rotation.is_finite());
    }

    #[test]
    fn orientation_publisher_emits_initial_orientation_once_and_then_stays_idle() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<CameraOrientationState>()
            .init_resource::<ViewportEventOutbox>()
            .add_systems(Update, publish_camera_orientation);
        app.world_mut()
            .spawn((Camera3d::default(), Transform::default()));

        app.update();
        let first = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("the initial live camera orientation is published");
        assert!(matches!(
            first.event,
            ViewportEvent::CameraOrientationChanged { orientation }
                if orientation == CameraOrientationReadModel::default()
        ));

        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<ViewportEventOutbox>()
                .pop()
                .is_none()
        );
    }

    #[test]
    fn orientation_publisher_drops_nonfinite_camera_rotations() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<CameraOrientationState>()
            .init_resource::<ViewportEventOutbox>()
            .add_systems(Update, publish_camera_orientation);
        app.world_mut().spawn((
            Camera3d::default(),
            Transform {
                rotation: Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0),
                ..Default::default()
            },
        ));

        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<ViewportEventOutbox>()
                .pop()
                .is_none()
        );
    }
}
