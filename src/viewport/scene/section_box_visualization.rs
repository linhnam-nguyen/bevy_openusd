//! Immediate-mode visualization for the aggregate Section Box.

use bevy::prelude::*;

use super::section_box::SectionBoxState;

const SECTION_BOX_COLOR: Color = Color::srgba(0.15, 0.75, 1.0, 0.9);

/// Draws exactly one wire box from the aggregate bounds. This is a transient
/// overlay only; the state remains the sole source for later clipping/gizmo
/// consumers and no helper entity is created per selected target.
pub(in crate::viewport) fn draw_section_box(state: Res<SectionBoxState>, mut gizmos: Gizmos) {
    if !state.enabled || !state.visible {
        return;
    }
    if state.bounds.is_none() {
        return;
    }
    for (start, end) in section_box_edges(state.transform) {
        gizmos.line(start, end, SECTION_BOX_COLOR);
    }
}

fn section_box_edges(transform: Transform) -> [(Vec3, Vec3); 12] {
    let matrix = transform.to_matrix();
    let corners = [
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
    ]
    .map(|corner| matrix.transform_point3(corner));
    [
        (corners[0], corners[1]),
        (corners[1], corners[2]),
        (corners[2], corners[3]),
        (corners[3], corners[0]),
        (corners[4], corners[5]),
        (corners[5], corners[6]),
        (corners[6], corners[7]),
        (corners[7], corners[4]),
        (corners[0], corners[4]),
        (corners[1], corners[5]),
        (corners[2], corners[6]),
        (corners[3], corners[7]),
    ]
}

#[cfg(test)]
mod tests {
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
}
