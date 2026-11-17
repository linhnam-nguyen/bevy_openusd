use super::*;

#[test]
fn aggregate_visualization_has_exactly_one_box() {
    let edges = section_box_edges(Transform::from_scale(Vec3::ONE));
    assert_eq!(edges.len(), 12);
}

#[test]
fn visualization_follows_the_interactive_box_transform() {
    let transform = Transform {
        translation: Vec3::new(3.0, 4.0, 5.0),
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        scale: Vec3::new(2.0, 4.0, 6.0),
    };
    let edges = section_box_edges(transform);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (start, end) in edges {
        min = min.min(start).min(end);
        max = max.max(start).max(end);
    }

    assert!(min.abs_diff_eq(Vec3::new(0.0, 2.0, 4.0), 0.0001));
    assert!(max.abs_diff_eq(Vec3::new(6.0, 6.0, 6.0), 0.0001));
}
