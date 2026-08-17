/// Draw a bright yellow AABB around the currently selected prim so the
/// user can visually locate the entity they clicked in the tree panel.
/// Draws a gizmo outline around the currently selected prim's bounds.
use bevy::prelude::*;

use super::SelectedPrim;

pub(crate) fn draw_selected_prim_highlight(
    selected: Res<SelectedPrim>,
    xforms: Query<&GlobalTransform>,
    aabbs: Query<&bevy::camera::primitives::Aabb>,
    mut gizmos: Gizmos,
) {
    let Some(entity) = selected.0 else {
        return;
    };
    let Ok(gt) = xforms.get(entity) else {
        return;
    };
    let origin = gt.translation();
    let color = Color::srgb(1.0, 0.9, 0.2);

    if let Ok(aabb) = aabbs.get(entity) {
        // Mesh AABB is in local space; transform corners into world.
        let half = Vec3::new(
            aabb.half_extents.x,
            aabb.half_extents.y,
            aabb.half_extents.z,
        );
        let centre_local = Vec3::new(aabb.center.x, aabb.center.y, aabb.center.z);
        let iso = gt.compute_transform();
        let corners = [
            Vec3::new(-half.x, -half.y, -half.z),
            Vec3::new(half.x, -half.y, -half.z),
            Vec3::new(half.x, half.y, -half.z),
            Vec3::new(-half.x, half.y, -half.z),
            Vec3::new(-half.x, -half.y, half.z),
            Vec3::new(half.x, -half.y, half.z),
            Vec3::new(half.x, half.y, half.z),
            Vec3::new(-half.x, half.y, half.z),
        ];
        let worldify = |v: Vec3| iso.translation + iso.rotation * ((v + centre_local) * iso.scale);
        let c: [Vec3; 8] = std::array::from_fn(|i| worldify(corners[i]));
        // 12 edges of the box.
        let edges = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0), // bottom
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4), // top
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7), // sides
        ];
        for (a, b) in edges {
            gizmos.line(c[a], c[b], color);
        }
    } else {
        // No AABB (no Mesh3d): fall back to a small cross.
        let l = 0.2;
        gizmos.line(origin - Vec3::X * l, origin + Vec3::X * l, color);
        gizmos.line(origin - Vec3::Y * l, origin + Vec3::Y * l, color);
        gizmos.line(origin - Vec3::Z * l, origin + Vec3::Z * l, color);
    }
}
