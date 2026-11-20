use bevy::math::{DQuat, DVec3};
use bevy::transform::components::Transform;

use super::{GizmoResult, core};

/// Applies a generic gizmo result to a Bevy target unless the host owns the
/// result's semantics, as it does for bounds-face resizing.
pub(super) fn apply_result_transform(
    target_transform: &mut Transform,
    result: GizmoResult,
    result_transform: core::math::Transform,
) {
    if matches!(result, GizmoResult::ResizeFace { .. }) {
        return;
    }

    target_transform.translation = DVec3::from(result_transform.translation).as_vec3();
    target_transform.rotation = DQuat::from(result_transform.rotation).as_quat();
    target_transform.scale = DVec3::from(result_transform.scale).as_vec3();
}

#[cfg(test)]
mod tests {
    use bevy::math::{DQuat, DVec3, Quat, Vec3};
    use bevy::transform::components::Transform as BevyTransform;

    use super::super::{BoundsFace, GizmoResult, core::math::Transform as GizmoTransform};
    use super::apply_result_transform;

    #[test]
    fn bounds_face_is_host_owned_while_translate_and_rotate_update_target() {
        let initial = BevyTransform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let result_transform = GizmoTransform::from_scale_rotation_translation(
            DVec3::splat(2.0),
            DQuat::from_rotation_y(0.5),
            DVec3::new(4.0, 5.0, 6.0),
        );

        let mut target = initial;
        apply_result_transform(
            &mut target,
            GizmoResult::ResizeFace {
                face: BoundsFace::PositiveX,
                delta: 1.0,
            },
            result_transform,
        );
        assert_eq!(target, initial);

        let mut translated = initial;
        apply_result_transform(
            &mut translated,
            GizmoResult::Translation {
                delta: mint::Vector3 {
                    x: 3.0,
                    y: 4.0,
                    z: 5.0,
                },
                total: mint::Vector3 {
                    x: 3.0,
                    y: 4.0,
                    z: 5.0,
                },
            },
            result_transform,
        );
        assert_eq!(translated.translation, Vec3::new(4.0, 5.0, 6.0));

        let mut rotated = initial;
        apply_result_transform(
            &mut rotated,
            GizmoResult::Rotation {
                axis: mint::Vector3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                delta: 0.5,
                total: 0.5,
                is_view_axis: false,
            },
            result_transform,
        );
        assert_eq!(
            rotated.rotation,
            DQuat::from(result_transform.rotation).as_quat()
        );
    }
}
