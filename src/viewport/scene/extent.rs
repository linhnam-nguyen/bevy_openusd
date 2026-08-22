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

#[derive(Clone, Copy)]
struct ExtentPrim {
    global: GlobalTransform,
    local: Option<usd_bevy::UsdLocalExtent>,
    aabb: Option<bevy::camera::primitives::Aabb>,
    is_geometry: bool,
}

fn recompute_scene_extent(prims: impl IntoIterator<Item = ExtentPrim>) -> SceneExtent {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut geometry_min = Vec3::splat(f32::INFINITY);
    let mut geometry_max = Vec3::splat(f32::NEG_INFINITY);
    let mut count = 0u32;
    let mut geometry_count = 0u32;
    for prim in prims {
        let mut prim_min = Vec3::splat(f32::INFINITY);
        let mut prim_max = Vec3::splat(f32::NEG_INFINITY);
        if let Some(le) = prim.local {
            let m = prim.global.to_matrix();
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
        } else if let Some(aabb) = prim.aabb {
            let m = prim.global.to_matrix();
            let center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            for i in 0..8 {
                let local = Vec3::new(
                    if i & 1 == 0 {
                        center.x - half.x
                    } else {
                        center.x + half.x
                    },
                    if i & 2 == 0 {
                        center.y - half.y
                    } else {
                        center.y + half.y
                    },
                    if i & 4 == 0 {
                        center.z - half.z
                    } else {
                        center.z + half.z
                    },
                );
                let w = m.transform_point3(local);
                prim_min = prim_min.min(w);
                prim_max = prim_max.max(w);
            }
        } else {
            let p = prim.global.translation();
            prim_min = prim_min.min(p);
            prim_max = prim_max.max(p);
        }
        min = min.min(prim_min);
        max = max.max(prim_max);
        if prim.is_geometry {
            geometry_min = geometry_min.min(prim_min);
            geometry_max = geometry_max.max(prim_max);
            geometry_count += 1;
        }
        count += 1;
    }
    SceneExtent {
        min,
        max,
        count,
        geometry_min,
        geometry_max,
        geometry_count,
    }
}

pub(crate) fn compute_extent(
    dirty_prims: Query<
        Entity,
        (
            With<UsdPrimRef>,
            Or<(
                Added<UsdPrimRef>,
                Changed<UsdPrimRef>,
                Changed<Transform>,
                Changed<GlobalTransform>,
                Changed<usd_bevy::UsdLocalExtent>,
                Changed<bevy::camera::primitives::Aabb>,
                Changed<Mesh3d>,
            )>,
        ),
    >,
    all_prims: Query<
        (
            &GlobalTransform,
            Option<&usd_bevy::UsdLocalExtent>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&Mesh3d>,
        ),
        With<UsdPrimRef>,
    >,
    mut removed_prims: RemovedComponents<UsdPrimRef>,
    mut removed_transforms: RemovedComponents<Transform>,
    mut removed_global_transforms: RemovedComponents<GlobalTransform>,
    mut removed_extents: RemovedComponents<usd_bevy::UsdLocalExtent>,
    mut removed_aabbs: RemovedComponents<bevy::camera::primitives::Aabb>,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut extent: ResMut<SceneExtent>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    let scene_changed = !dirty_prims.is_empty()
        || removed_prims.read().next().is_some()
        || removed_transforms.read().next().is_some()
        || removed_global_transforms.read().next().is_some()
        || removed_extents.read().next().is_some()
        || removed_aabbs.read().next().is_some()
        || removed_meshes.read().next().is_some();
    if !scene_changed {
        return;
    }

    if let Some(ref mut c) = counters {
        c.grid_compute_extent_calls += 1;
    }
    let next_extent = recompute_scene_extent(all_prims.iter().map(
        |(global, local, aabb, mesh)| ExtentPrim {
            global: *global,
            local: local.copied(),
            aabb: aabb.copied(),
            is_geometry: mesh.is_some(),
        },
    ));
    if let Some(ref mut c) = counters {
        c.grid_prims_scanned += next_extent.count as u64;
    }
    *extent = next_extent;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prim(x: f32) -> ExtentPrim {
        ExtentPrim {
            global: GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0)),
            local: Some(usd_bevy::UsdLocalExtent {
                min: [-1.0, -1.0, -1.0],
                max: [1.0, 1.0, 1.0],
            }),
            aabb: None,
            is_geometry: true,
        }
    }

    #[test]
    fn transformed_prim_recompute_keeps_unchanged_global_bounds() {
        let extent = recompute_scene_extent([prim(-100.0), prim(0.0), prim(100.0)]);
        let moved = prim(25.0);
        let updated = recompute_scene_extent([prim(-100.0), moved, prim(100.0)]);

        assert_eq!(extent.count, 3);
        assert_eq!(updated.count, 3);
        assert_eq!(updated.min, Vec3::new(-101.0, -1.0, -1.0));
        assert_eq!(updated.max, Vec3::new(101.0, 1.0, 1.0));
    }

    #[test]
    fn add_and_remove_recompute_over_all_current_prims() {
        let added = prim(300.0);
        let with_added = recompute_scene_extent([prim(-100.0), prim(100.0), added]);
        let after_remove = recompute_scene_extent([prim(-100.0), added]);

        assert_eq!(with_added.count, 3);
        assert_eq!(with_added.max.x, 301.0);
        assert_eq!(after_remove.count, 2);
        assert_eq!(after_remove.min.x, -101.0);
        assert_eq!(after_remove.max.x, 301.0);
    }

    #[test]
    fn removing_last_prim_returns_empty_extent() {
        let empty = recompute_scene_extent(std::iter::empty());

        assert_eq!(empty, SceneExtent::default());
    }

    #[test]
    fn reload_replaces_previous_stage_bounds() {
        let reloaded = recompute_scene_extent([prim(-7.0), prim(12.0)]);

        assert_eq!(reloaded.count, 2);
        assert_eq!(reloaded.min.x, -8.0);
        assert_eq!(reloaded.max.x, 13.0);
        assert_eq!(reloaded.geometry_count, 2);
    }

    #[test]
    fn local_extent_uses_world_transform_for_root_or_z_up_scene() {
        let mut root = prim(0.0);
        root.global = GlobalTransform::from(
            Transform::from_xyz(10.0, 20.0, 30.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        );
        let extent = recompute_scene_extent([root]);

        assert_eq!(extent.count, 1);
        assert_eq!(extent.min, Vec3::new(9.0, 19.0, 29.0));
        assert_eq!(extent.max, Vec3::new(11.0, 21.0, 31.0));
    }
}
