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
        StandardView::Right => (-core::f32::consts::FRAC_PI_2, 0.0),
        StandardView::Left => (core::f32::consts::FRAC_PI_2, 0.0),
        // At the poles apply_rig selects a stable alternate up axis.
        StandardView::Top => (0.0, core::f32::consts::FRAC_PI_2),
        StandardView::Bottom => (0.0, -core::f32::consts::FRAC_PI_2),
    }
}

#[cfg(test)]
mod tests {
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
            (-core::f32::consts::FRAC_PI_2, 0.0)
        );
        assert_eq!(
            standard_view_angles(StandardView::Left),
            (core::f32::consts::FRAC_PI_2, 0.0)
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
}
