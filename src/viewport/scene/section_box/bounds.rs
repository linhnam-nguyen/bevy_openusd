//! Renderer-local Section Box bounds resolution for fallback callers.

use std::collections::HashSet;

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use super::SectionBoxBounds;
use crate::viewport::api::SceneAnchorIndex;

pub(in crate::viewport) fn aggregate_selection_bounds(
    targets: &[SceneAnchor],
    scene_index: &SceneAnchorIndex,
    renderables: &Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<SectionBoxBounds> {
    let mut aggregate: Option<SectionBoxBounds> = None;
    for target in targets {
        let Some(root) = scene_index.resolve(target) else {
            continue;
        };
        let Some(target_bounds) = target_bounds(root, renderables) else {
            continue;
        };
        if let Some(current) = &mut aggregate {
            current.include(target_bounds);
        } else {
            aggregate = Some(target_bounds);
        }
    }
    aggregate
}

fn target_bounds(
    root: Entity,
    renderables: &Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<SectionBoxBounds> {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    let mut aggregate: Option<SectionBoxBounds> = None;
    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) {
            continue;
        }
        let Ok((global, children, mesh, aabb, local_extent)) = renderables.get(entity) else {
            continue;
        };
        if let (Some(global), Some(_mesh)) = (global, mesh) {
            let local_bounds = local_extent
                .map(|extent| SectionBoxBounds {
                    min: Vec3::from_array(extent.min),
                    max: Vec3::from_array(extent.max),
                })
                .or_else(|| {
                    aabb.map(|aabb| {
                        let center = Vec3::from(aabb.center);
                        let half_extents = Vec3::from(aabb.half_extents);
                        SectionBoxBounds {
                            min: center - half_extents,
                            max: center + half_extents,
                        }
                    })
                });
            if let Some(local_bounds) = local_bounds {
                let world_bounds = transform_bounds(local_bounds, global.to_matrix());
                if let Some(current) = &mut aggregate {
                    current.include(world_bounds);
                } else {
                    aggregate = Some(world_bounds);
                }
            }
        }
        if let Some(children) = children {
            stack.extend(children.iter());
        }
    }
    aggregate
}

fn transform_bounds(bounds: SectionBoxBounds, matrix: Mat4) -> SectionBoxBounds {
    let mut transformed = SectionBoxBounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    for index in 0..8 {
        let corner = Vec3::new(
            if index & 1 == 0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            if index & 2 == 0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            if index & 4 == 0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        );
        let world = matrix.transform_point3(corner);
        transformed.min = transformed.min.min(world);
        transformed.max = transformed.max.max(world);
    }
    transformed
}
