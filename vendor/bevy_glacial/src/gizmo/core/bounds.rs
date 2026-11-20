use glam::{DQuat, DVec3};

use super::math::Transform;

/// One of the six faces of a generic oriented bounds gizmo.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BoundsFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl BoundsFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    pub const fn axis_index(self) -> usize {
        match self {
            Self::PositiveX | Self::NegativeX => 0,
            Self::PositiveY | Self::NegativeY => 1,
            Self::PositiveZ | Self::NegativeZ => 2,
        }
    }

    pub const fn local_axis(self) -> DVec3 {
        match self {
            Self::PositiveX | Self::NegativeX => DVec3::X,
            Self::PositiveY | Self::NegativeY => DVec3::Y,
            Self::PositiveZ | Self::NegativeZ => DVec3::Z,
        }
    }

    pub const fn sign(self) -> f64 {
        match self {
            Self::PositiveX | Self::PositiveY | Self::PositiveZ => 1.0,
            Self::NegativeX | Self::NegativeY | Self::NegativeZ => -1.0,
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::PositiveX => Self::NegativeX,
            Self::NegativeX => Self::PositiveX,
            Self::PositiveY => Self::NegativeY,
            Self::NegativeY => Self::PositiveY,
            Self::PositiveZ => Self::NegativeZ,
            Self::NegativeZ => Self::PositiveZ,
        }
    }
}

/// Renderer-safe lower bound for one dimension of a bounds gizmo.
pub const DEFAULT_BOUNDS_MIN_THICKNESS: f64 = 0.001;

/// Resizes exactly one face of an oriented bounds transform.
///
/// `delta` is signed along the selected face's outward local normal. Positive
/// values extend the box and negative values contract it. The opposite face
/// remains fixed, including when the transform is rotated.
pub fn resize_transform_face(
    transform: Transform,
    face: BoundsFace,
    delta: f64,
    min_thickness: f64,
) -> Transform {
    let mut size = DVec3::from(transform.scale).abs();
    let axis = face.axis_index();
    let old_size = size[axis];
    let new_size = (old_size + delta).max(min_thickness);
    let effective_delta = new_size - old_size;
    size[axis] = new_size;

    let normal_world = DQuat::from(transform.rotation) * (face.local_axis() * face.sign());
    let translation = DVec3::from(transform.translation) + normal_world * (effective_delta * 0.5);

    Transform {
        scale: size.into(),
        rotation: transform.rotation,
        translation: translation.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    fn face_plane(transform: Transform, face: BoundsFace) -> (DVec3, DVec3) {
        let normal = DQuat::from(transform.rotation) * (face.local_axis() * face.sign());
        let half_size = DVec3::from(transform.scale).abs()[face.axis_index()] * 0.5;
        let point = DVec3::from(transform.translation) + normal * half_size;
        (normal, point)
    }

    fn assert_plane_unchanged(before: Transform, after: Transform, face: BoundsFace) {
        let (before_normal, before_point) = face_plane(before, face);
        let (after_normal, after_point) = face_plane(after, face);
        assert!((before_normal - after_normal).length() < 1e-10);
        assert!((before_normal.dot(before_point) - after_normal.dot(after_point)).abs() < 1e-10);
    }

    #[test]
    fn every_face_extends_and_contracts_one_side_only() {
        let before = Transform::from_scale_rotation_translation(
            DVec3::new(10.0, 12.0, 14.0),
            DQuat::IDENTITY,
            DVec3::ZERO,
        );

        for face in BoundsFace::ALL {
            let extended = resize_transform_face(before, face, 2.0, DEFAULT_BOUNDS_MIN_THICKNESS);
            let contracted =
                resize_transform_face(before, face, -2.0, DEFAULT_BOUNDS_MIN_THICKNESS);

            assert_eq!(extended.rotation, before.rotation);
            assert_eq!(contracted.rotation, before.rotation);
            assert_plane_unchanged(before, extended, face.opposite());
            assert_plane_unchanged(before, contracted, face.opposite());

            for other in BoundsFace::ALL {
                if other != face && other != face.opposite() {
                    assert_plane_unchanged(before, extended, other);
                    assert_plane_unchanged(before, contracted, other);
                }
            }

            let (normal, before_point) = face_plane(before, face);
            let (_, extended_point) = face_plane(extended, face);
            assert!((extended_point - before_point - normal * 2.0).length() < 1e-10);
        }
    }

    #[test]
    fn rotated_face_drag_uses_local_normal_and_keeps_opposite_plane_fixed() {
        let before = Transform::from_scale_rotation_translation(
            DVec3::splat(10.0),
            DQuat::from_rotation_y(0.7) * DQuat::from_rotation_z(-0.35),
            DVec3::new(4.0, -2.0, 3.0),
        );
        let (opposite_normal, opposite_point) =
            face_plane(before, BoundsFace::PositiveX.opposite());
        let after = resize_transform_face(before, BoundsFace::PositiveX, -3.0, 0.001);

        assert!(
            (DVec3::from(after.translation)
                - DVec3::from(before.translation)
                - opposite_normal * 1.5)
                .length()
                < 1e-10
        );
        let (_, after_opposite_point) = face_plane(after, BoundsFace::PositiveX.opposite());
        assert!((opposite_point - after_opposite_point).length() < 1e-10);
        assert_eq!(after.rotation, before.rotation);
    }

    #[test]
    fn contraction_clamps_to_minimum_without_crossing() {
        let before = Transform::from_scale_rotation_translation(
            DVec3::new(2.0, 3.0, 4.0),
            DQuat::IDENTITY,
            DVec3::ZERO,
        );
        let after = resize_transform_face(before, BoundsFace::NegativeY, -10.0, 0.001);

        assert!((DVec3::from(after.scale) - DVec3::new(2.0, 0.001, 4.0)).length() < 1e-10);
        assert!((DVec3::from(after.translation) - DVec3::new(0.0, 1.4995, 0.0)).length() < 1e-10);
        assert!(DVec3::from(after.scale).y >= 0.001);
    }

    #[test]
    fn incremental_face_drags_and_restore_are_stable() {
        let before = Transform::from_scale_rotation_translation(
            DVec3::splat(8.0),
            DQuat::IDENTITY,
            DVec3::ZERO,
        );
        let first = resize_transform_face(before, BoundsFace::PositiveZ, -1.0, 0.001);
        let second = resize_transform_face(first, BoundsFace::PositiveZ, 1.0, 0.001);
        assert!((DVec3::from(second.scale) - DVec3::from(before.scale)).length() < 1e-10);
        assert!(
            (DVec3::from(second.translation) - DVec3::from(before.translation)).length() < 1e-10
        );
    }
}
