//! World-space scene bounding extent and geometry ground reference calculation.

use bevy::prelude::*;
use usd_bevy::UsdPrimRef;

/// Tracks the bounding box of all loaded USD prims in world space.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SceneExtent {
    pub min: Vec3,
    pub max: Vec3,
    pub count: u32,
    pub geometry_min: Vec3,
    pub geometry_max: Vec3,
    pub geometry_count: u32,
}

impl Default for SceneExtent {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
            count: 0,
            geometry_min: Vec3::splat(f32::INFINITY),
            geometry_max: Vec3::splat(f32::NEG_INFINITY),
            geometry_count: 0,
        }
    }
}

impl SceneExtent {
    /// Longest diagonal across the scene AABB, or `1.0` if empty.
    pub fn diag(&self) -> f32 {
        if self.count == 0 {
            1.0
        } else {
            (self.max - self.min).length().max(0.01)
        }
    }

    /// Center of the scene AABB in world space, or `(0, 0, 0)` if empty.
    pub fn center(&self) -> Vec3 {
        if self.count == 0 {
            Vec3::ZERO
        } else {
            (self.min + self.max) * 0.5
        }
    }

    /// Alias for center (UK/US spelling compatibility).
    pub fn centre(&self) -> Vec3 {
        self.center()
    }

    /// Returns a scene-derived ground reference just above the lowest
    /// renderable geometry. This intentionally ignores lights, cameras,
    /// Xforms, and the synthetic stage root.
    pub fn geometry_ground_y(&self) -> Option<f32> {
        if self.geometry_count == 0 {
            return None;
        }
        let geometry_diag = (self.geometry_max - self.geometry_min).length().max(0.01);
        let lift = (geometry_diag * 0.0005).clamp(0.0001, 0.05);
        Some(self.geometry_min.y + lift)
    }
}

pub(crate) fn compute_extent(
    prims: Query<
        (
            &GlobalTransform,
            Option<&usd_bevy::UsdLocalExtent>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&Mesh3d>,
        ),
        With<UsdPrimRef>,
    >,
    mut extent: ResMut<SceneExtent>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    if let Some(ref mut c) = counters {
        c.grid_compute_extent_calls += 1;
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut geometry_min = Vec3::splat(f32::INFINITY);
    let mut geometry_max = Vec3::splat(f32::NEG_INFINITY);
    let mut count = 0u32;
    let mut geometry_count = 0u32;
    for (gt, local, aabb, mesh) in prims.iter() {
        let mut prim_min = Vec3::splat(f32::INFINITY);
        let mut prim_max = Vec3::splat(f32::NEG_INFINITY);
        if let Some(le) = local {
            let m = gt.to_matrix();
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { le.min[0] } else { le.max[0] },
                    if i & 2 == 0 { le.min[1] } else { le.max[1] },
                    if i & 4 == 0 { le.min[2] } else { le.max[2] },
                );
                let w = m.transform_point3(c);
                prim_min = prim_min.min(w);
                prim_max = prim_max.max(w);
            }
        } else if let Some(aabb) = aabb {
            let m = gt.to_matrix();
            let center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            for i in 0..8 {
                let local = Vec3::new(
                    if i & 1 == 0 { center.x - half.x } else { center.x + half.x },
                    if i & 2 == 0 { center.y - half.y } else { center.y + half.y },
                    if i & 4 == 0 { center.z - half.z } else { center.z + half.z },
                );
                let w = m.transform_point3(local);
                prim_min = prim_min.min(w);
                prim_max = prim_max.max(w);
            }
        } else {
            let p = gt.translation();
            prim_min = prim_min.min(p);
            prim_max = prim_max.max(p);
        }
        min = min.min(prim_min);
        max = max.max(prim_max);
        if mesh.is_some() {
            geometry_min = geometry_min.min(prim_min);
            geometry_max = geometry_max.max(prim_max);
            geometry_count += 1;
        }
        count += 1;
    }
    *extent = SceneExtent {
        min,
        max,
        count,
        geometry_min,
        geometry_max,
        geometry_count,
    };
}
