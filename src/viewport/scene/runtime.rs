//! Runtime presentation systems for the Bevy projection of the active stage.

use bevy::asset::Assets;
use bevy::prelude::*;
use usd_bevy::{UsdAsset, UsdPrimRef};

use super::SelectedPrim;
use crate::viewport::session::{LoaderTuning, StageHandle};

/// Live curve / point tuning. On every `CurveTuning` change (slider
/// move is enough), iterate the prim tree, look up each curve/points
/// prim's raw data on the loaded `UsdAsset`, and rebuild the mesh's
/// vertex buffers in place via `Assets<Mesh>::get_mut`. No asset
/// reload, no AssetServer-cache fight.
/// Rebuilds curve and point meshes when live loader tuning changes.
pub(crate) fn rebuild_tuned_meshes(
    tuning: Res<LoaderTuning>,
    stage: Option<Res<StageHandle>>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    prims: Query<(&UsdPrimRef, &bevy::mesh::Mesh3d)>,
    mut last: Local<Option<(f32, u32, f32)>>,
) {
    let Some(stage) = stage else { return };
    let Some(asset) = usd_assets.get(&stage.0) else {
        return;
    };
    let radius = tuning.curves.default_radius;
    let rings = tuning.curves.ring_segments;
    let point_scale = tuning.curves.point_scale;
    // egui's ResMut access fires `is_changed` every frame, so compare
    // the actual slider values — only rebuild when they really move.
    let key = (radius, rings, point_scale);
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    let mut rebuilt = 0usize;

    for (prim, mesh3d) in prims.iter() {
        if let Some(read) = asset.curves.get(&prim.path) {
            let new_mesh = usd_bevy::curves::curves_mesh(read, radius, rings);
            if let Some(mut slot) = meshes.get_mut(&mesh3d.0) {
                *slot = new_mesh;
                rebuilt += 1;
            }
        } else if let Some(read) = asset.points_clouds.get(&prim.path) {
            let new_mesh = usd_bevy::curves::points_mesh(read, point_scale);
            if let Some(mut slot) = meshes.get_mut(&mesh3d.0) {
                *slot = new_mesh;
                rebuilt += 1;
            }
        }
    }
    if rebuilt > 0 {
        info!(
            "tuning: rebuilt {rebuilt} curve/point mesh(es) (radius={radius:.4}, rings={rings}, point_scale={point_scale:.2})"
        );
    }
}

/// Draw a bright yellow AABB around the currently selected prim so the
/// user can visually locate the entity they clicked in the tree panel.
/// Draws a gizmo outline around the currently selected prim's bounds.
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
