//! World-space bounds projection for selected renderables.

use std::collections::{HashMap, HashSet};

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProjectedWorldBounds {
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
}

impl ProjectedWorldBounds {
    pub(super) fn include(&mut self, other: Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }
}

pub(super) fn collect_mesh_descendants(
    root: Entity,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
) -> HashSet<Entity> {
    let mut output = HashSet::new();
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) {
            continue;
        }
        let Ok((children, mesh)) = hierarchy.get(entity) else {
            continue;
        };
        if mesh.is_some() {
            output.insert(entity);
        }
        if let Some(children) = children {
            stack.extend(children.iter());
        }
    }
    output
}

pub(super) fn replace_target_bounds(
    projection: &mut super::SelectedRenderableProjection,
    target: &SceneAnchor,
    bounds: Option<ProjectedWorldBounds>,
) {
    projection.target_bounds.remove(target);
    if let Some(bounds) = bounds {
        projection.target_bounds.insert(target.clone(), bounds);
    }
}

pub(super) fn bounds_for_entities(
    entities: &HashSet<Entity>,
    geometry: &Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<ProjectedWorldBounds> {
    let mut aggregate: Option<ProjectedWorldBounds> = None;
    for entity in entities {
        let Ok((global, mesh, aabb, local_extent)) = geometry.get(*entity) else {
            continue;
        };
        let (Some(global), Some(_mesh)) = (global, mesh) else {
            continue;
        };
        let local = local_extent
            .map(|extent| ProjectedWorldBounds {
                min: Vec3::from_array(extent.min),
                max: Vec3::from_array(extent.max),
            })
            .or_else(|| {
                aabb.map(|aabb| {
                    let center = Vec3::from(aabb.center);
                    let half_extents = Vec3::from(aabb.half_extents);
                    ProjectedWorldBounds {
                        min: center - half_extents,
                        max: center + half_extents,
                    }
                })
            });
        let Some(local) = local else {
            continue;
        };
        let world = transform_bounds(local, global.to_matrix());
        if let Some(current) = &mut aggregate {
            current.include(world);
        } else {
            aggregate = Some(world);
        }
    }
    aggregate
}

pub(super) fn aggregate_bounds(
    target_bounds: &HashMap<SceneAnchor, ProjectedWorldBounds>,
) -> Option<ProjectedWorldBounds> {
    target_bounds
        .values()
        .copied()
        .reduce(|mut aggregate, next| {
            aggregate.include(next);
            aggregate
        })
}

fn transform_bounds(bounds: ProjectedWorldBounds, matrix: Mat4) -> ProjectedWorldBounds {
    let mut transformed = ProjectedWorldBounds {
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
