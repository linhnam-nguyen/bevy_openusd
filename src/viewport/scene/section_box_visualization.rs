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
#[path = "../../../tests/unit/viewport/scene/section_box_visualization_inline_tests.rs"]
mod tests;
