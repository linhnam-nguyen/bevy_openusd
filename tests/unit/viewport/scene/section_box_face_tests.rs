use super::*;

fn face_plane(transform: Transform, face: SectionBoxFace) -> (Vec3, f32) {
    let normal = transform.rotation * (face.local_axis() * face.sign());
    let point = transform.translation + normal * (transform.scale.abs()[face.axis_index()] * 0.5);
    (normal, normal.dot(point))
}

fn assert_plane_eq(before: Transform, after: Transform, face: SectionBoxFace) {
    let (before_normal, before_offset) = face_plane(before, face);
    let (after_normal, after_offset) = face_plane(after, face);
    assert!((before_normal - after_normal).length() < 1e-5);
    assert!((before_offset - after_offset).abs() < 1e-5);
}

#[test]
fn every_face_contracts_and_extends_without_moving_other_faces() {
    let before = Transform {
        translation: Vec3::new(4.0, -3.0, 2.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(10.0, 12.0, 14.0),
    };

    for face in SectionBoxFace::ALL {
        for displacement in [-2.0, 2.0] {
            let after = resize_section_box_face(before, face, displacement);
            let expected_size = before.scale[face.axis_index()] + displacement;
            assert!((after.scale[face.axis_index()] - expected_size).abs() < 1e-5);
            assert_eq!(after.rotation, before.rotation);
            assert!(
                (after.scale
                    - before.scale
                    - face.local_axis() * (expected_size - before.scale[face.axis_index()]))
                .length()
                    < 1e-5
            );
            assert_plane_eq(before, after, face.opposite());
            for other in SectionBoxFace::ALL {
                if other != face && other != face.opposite() {
                    assert_plane_eq(before, after, other);
                }
            }
        }
    }
}

#[test]
fn rotated_face_drag_uses_local_normal_for_all_axis_pairs() {
    let before = Transform {
        translation: Vec3::new(4.0, -3.0, 2.0),
        rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_z(-0.35),
        scale: Vec3::splat(10.0),
    };

    for face in SectionBoxFace::ALL {
        let opposite = face.opposite();
        let (_, before_opposite_offset) = face_plane(before, opposite);
        let after = resize_section_box_face(before, face, -3.0);
        let (_, after_opposite_offset) = face_plane(after, opposite);
        assert!((before_opposite_offset - after_opposite_offset).abs() < 1e-5);
        assert_eq!(after.rotation, before.rotation);
    }
}

#[test]
fn minimum_thickness_clamps_crossing_and_uses_effective_delta() {
    let before =
        Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)).with_scale(Vec3::new(2.0, 3.0, 4.0));
    let after = resize_section_box_face(before, SectionBoxFace::NegativeY, -10.0);

    assert!((after.scale - Vec3::new(2.0, MIN_SECTION_BOX_THICKNESS, 4.0)).length() < 1e-5);
    assert!((after.translation - Vec3::new(1.0, 3.4995, 3.0)).length() < 1e-5);
    assert!(after.scale.y >= MIN_SECTION_BOX_THICKNESS);
}

#[test]
fn repeated_incremental_drag_and_reverse_restore_the_original_pose() {
    let before = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)).with_scale(Vec3::splat(8.0));
    let adjusted = resize_section_box_face(before, SectionBoxFace::PositiveZ, -1.0);
    let restored = resize_section_box_face(adjusted, SectionBoxFace::PositiveZ, 1.0);

    assert!((restored.translation - before.translation).length() < 1e-5);
    assert!((restored.scale - before.scale).length() < 1e-5);
    assert_eq!(restored.rotation, before.rotation);
}
