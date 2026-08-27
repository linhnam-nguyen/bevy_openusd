use viewport_protocol::{CameraSource, StandardView, ViewportEvent, ViewportEventEnvelope};

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::camera::CameraMount;
use crate::viewport::camera::{ArcballCamera, FlyTo};

use super::super::helpers::reject;

pub(super) fn set_camera_source(
    request_id: String,
    source: CameraSource,
    outbox: &mut ViewportEventOutbox,
    camera_mount: &mut CameraMount,
) {
    *camera_mount = match &source {
        CameraSource::Arcball => CameraMount::Arcball,
        CameraSource::Authored { prim_path } => CameraMount::Mounted {
            prim_path: prim_path.clone(),
        },
    };
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::CameraSourceChanged { source },
    ));
}

pub(super) fn set_standard_view(
    request_id: String,
    view: StandardView,
    outbox: &mut ViewportEventOutbox,
    camera_mount: &mut CameraMount,
    fly_to: &mut FlyTo,
    cameras: &bevy::prelude::Query<'_, '_, &ArcballCamera>,
) {
    let Ok(camera) = cameras.single() else {
        reject(
            outbox,
            request_id,
            "standard view requires an active viewport camera".to_owned(),
        );
        return;
    };
    let (target_yaw, target_elevation) = standard_view_angles(view);
    let was_mounted = matches!(camera_mount, CameraMount::Mounted { .. });
    *camera_mount = CameraMount::Arcball;
    *fly_to = FlyTo {
        target_focus: camera.focus,
        target_distance: camera.distance,
        remaining: 0.18,
        duration: 0.18,
        start_focus: camera.focus,
        start_distance: camera.distance,
        start_yaw: Some(camera.yaw),
        target_yaw: Some(target_yaw),
        start_elevation: Some(camera.elevation),
        target_elevation: Some(target_elevation),
    };
    if was_mounted {
        outbox.push(ViewportEventEnvelope::new(
            Some(request_id.clone()),
            ViewportEvent::CameraSourceChanged {
                source: CameraSource::Arcball,
            },
        ));
    }
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::CameraStandardViewStarted { view },
    ));
}

fn standard_view_angles(view: StandardView) -> (f32, f32) {
    match view {
        // Camera offset directions: Front -Z, Back +Z, Right -X, Left +X.
        StandardView::Front => (0.0, 0.0),
        StandardView::Back => (core::f32::consts::PI, 0.0),
        StandardView::Right => (core::f32::consts::FRAC_PI_2, 0.0),
        StandardView::Left => (-core::f32::consts::FRAC_PI_2, 0.0),
        // At the poles apply_rig selects the canonical alternate up axis.
        StandardView::Top => (0.0, core::f32::consts::FRAC_PI_2),
        StandardView::Bottom => (0.0, -core::f32::consts::FRAC_PI_2),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::{Transform, Vec3};

    use super::*;

    #[test]
    fn standard_view_mapping_uses_canonical_camera_offsets() {
        assert_eq!(standard_view_angles(StandardView::Front), (0.0, 0.0));
        assert_eq!(
            standard_view_angles(StandardView::Back),
            (core::f32::consts::PI, 0.0)
        );
        assert_eq!(
            standard_view_angles(StandardView::Right),
            (core::f32::consts::FRAC_PI_2, 0.0)
        );
        assert_eq!(
            standard_view_angles(StandardView::Left),
            (-core::f32::consts::FRAC_PI_2, 0.0)
        );
        assert_eq!(
            standard_view_angles(StandardView::Top),
            (0.0, core::f32::consts::FRAC_PI_2)
        );
        assert_eq!(
            standard_view_angles(StandardView::Bottom),
            (0.0, -core::f32::consts::FRAC_PI_2)
        );
    }

    #[test]
    fn standard_view_rig_matches_each_canonical_camera_basis() {
        let cases = [
            (StandardView::Front, Vec3::NEG_Z, Vec3::Y, Vec3::X),
            (StandardView::Back, Vec3::Z, Vec3::Y, Vec3::NEG_X),
            (StandardView::Right, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
            (StandardView::Left, Vec3::X, Vec3::Y, Vec3::Z),
            (StandardView::Top, Vec3::NEG_Y, Vec3::NEG_Z, Vec3::X),
            (StandardView::Bottom, Vec3::Y, Vec3::Z, Vec3::X),
        ];

        for (view, expected_forward, expected_up, expected_right) in cases {
            let (yaw, elevation) = standard_view_angles(view);
            let camera = ArcballCamera {
                yaw,
                elevation,
                ..Default::default()
            };
            let mut transform = Transform::default();
            crate::viewport::camera::apply_rig(&camera, &mut transform);
            let actual_forward = transform.forward().as_vec3();
            let actual_up = transform.up().as_vec3();
            let actual_right = transform.right().as_vec3();

            assert!(
                actual_forward.distance(expected_forward) < 1e-4,
                "{view:?} produced {actual_forward:?}, expected {expected_forward:?}"
            );
            assert!(
                actual_up.distance(expected_up) < 1e-4,
                "{view:?} up {actual_up:?}, expected {expected_up:?}"
            );
            assert!(
                actual_right.distance(expected_right) < 1e-4,
                "{view:?} right {actual_right:?}, expected {expected_right:?}"
            );
        }
    }
}
