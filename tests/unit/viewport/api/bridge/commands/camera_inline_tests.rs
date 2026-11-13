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
