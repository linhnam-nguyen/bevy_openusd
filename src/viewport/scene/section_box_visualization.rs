//! Immediate-mode visualization for the aggregate Section Box.

use bevy::prelude::*;

use super::section_box::{SectionBoxBounds, SectionBoxState};

const SECTION_BOX_COLOR: Color = Color::srgba(0.15, 0.75, 1.0, 0.9);

/// Draws exactly one wire box from the aggregate bounds. This is a transient
/// overlay only; the state remains the sole source for later clipping/gizmo
/// consumers and no helper entity is created per selected target.
pub(in crate::viewport) fn draw_section_box(state: Res<SectionBoxState>, mut gizmos: Gizmos) {
    if !state.enabled || !state.visible {
        return;
    }
    let Some(bounds) = state.bounds else {
        return;
    };
    for (start, end) in section_box_edges(bounds) {
        gizmos.line(start, end, SECTION_BOX_COLOR);
    }
}

fn section_box_edges(bounds: SectionBoxBounds) -> [(Vec3, Vec3); 12] {
    let corners = [
        Vec3::new(bounds.min.x, bounds.min.y, bounds.min.z),
        Vec3::new(bounds.max.x, bounds.min.y, bounds.min.z),
        Vec3::new(bounds.max.x, bounds.max.y, bounds.min.z),
        Vec3::new(bounds.min.x, bounds.max.y, bounds.min.z),
        Vec3::new(bounds.min.x, bounds.min.y, bounds.max.z),
        Vec3::new(bounds.max.x, bounds.min.y, bounds.max.z),
        Vec3::new(bounds.max.x, bounds.max.y, bounds.max.z),
        Vec3::new(bounds.min.x, bounds.max.y, bounds.max.z),
    ];
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
        let edges = section_box_edges(SectionBoxBounds {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        });
        assert_eq!(edges.len(), 12);
    }
}
