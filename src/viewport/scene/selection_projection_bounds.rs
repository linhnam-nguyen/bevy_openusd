//! World-space bounds projection for selected renderables.

use std::collections::HashMap;

use bevy::camera::primitives::Aabb;
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

#[derive(Debug)]
pub(super) struct PendingTargetBounds {
    pub(super) target: SceneAnchor,
    next_renderable: usize,
    aggregate: Option<ProjectedWorldBounds>,
}

/// Starts exactly one resumable bounds pass per selected target. Repeated
/// topology admissions retain the cursor, so a growing selection cannot reset
/// a large aggregate scan every update.
pub(super) fn schedule_target_bounds(
    projection: &mut super::SelectedRenderableProjection,
    target: &SceneAnchor,
) -> bool {
    if projection
        .pending_bounds
        .iter()
        .any(|pending| &pending.target == target)
    {
        return false;
    }
    let had_bounds = projection.target_bounds.remove(target).is_some();
    if projection.target_renderables.contains_key(target) {
        projection.pending_bounds.push(PendingTargetBounds {
            target: target.clone(),
            next_renderable: 0,
            aggregate: None,
        });
    }
    had_bounds
}

pub(super) fn advance_pending_bounds(
    projection: &mut super::SelectedRenderableProjection,
    geometry: &Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
    mut budget: usize,
) -> bool {
    let mut changed = false;
    while budget > 0 {
        enum BoundsStep {
            Renderable {
                target: SceneAnchor,
                entity: Entity,
            },
            Finished {
                target: SceneAnchor,
                bounds: Option<ProjectedWorldBounds>,
            },
        }

        let step = {
            let Some(work) = projection.pending_bounds.last_mut() else {
                break;
            };
            let next = projection
                .target_renderable_order
                .get(&work.target)
                .and_then(|order| order.get(work.next_renderable))
                .copied();
            if let Some(entity) = next {
                work.next_renderable += 1;
                BoundsStep::Renderable {
                    target: work.target.clone(),
                    entity,
                }
            } else {
                BoundsStep::Finished {
                    target: work.target.clone(),
                    bounds: work.aggregate,
                }
            }
        };

        match step {
            BoundsStep::Renderable { target, entity } => {
                budget -= 1;
                let still_selected = projection
                    .target_renderables
                    .get(&target)
                    .is_some_and(|renderables| renderables.contains(&entity));
                if !still_selected {
                    continue;
                }
                let Some(bounds) = bounds_for_entity(entity, geometry) else {
                    continue;
                };
                let work = projection
                    .pending_bounds
                    .last_mut()
                    .expect("the active bounds cursor remains queued");
                if let Some(aggregate) = &mut work.aggregate {
                    aggregate.include(bounds);
                } else {
                    work.aggregate = Some(bounds);
                }
            }
            BoundsStep::Finished { target, bounds } => {
                projection.pending_bounds.pop();
                changed |= replace_target_bounds(projection, &target, bounds);
            }
        }
    }
    changed
}

fn replace_target_bounds(
    projection: &mut super::SelectedRenderableProjection,
    target: &SceneAnchor,
    bounds: Option<ProjectedWorldBounds>,
) -> bool {
    let previous = projection.target_bounds.get(target).copied();
    if previous == bounds {
        return false;
    }
    projection.target_bounds.remove(target);
    if let Some(bounds) = bounds {
        projection.target_bounds.insert(target.clone(), bounds);
    }
    true
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

fn bounds_for_entity(
    entity: Entity,
    geometry: &Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<ProjectedWorldBounds> {
    let Ok((global, mesh, aabb, local_extent)) = geometry.get(entity) else {
        return None;
    };
    let (Some(global), Some(_mesh)) = (global, mesh) else {
        return None;
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
        })?;
    Some(transform_bounds(local, global.to_matrix()))
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
