use std::collections::{HashMap, HashSet};

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use super::bounds::{bounds_for_entities, replace_target_bounds};
use super::{PendingTargetProjection, PendingTargetRemoval, SelectedRenderableProjection};

pub(super) fn schedule_targets(
    targets: &[SceneAnchor],
    scene_index: &super::SceneAnchorIndex,
    projection: &mut SelectedRenderableProjection,
) {
    for target in targets {
        schedule_target(target, scene_index, projection);
    }
}

fn schedule_target(
    target: &SceneAnchor,
    scene_index: &super::SceneAnchorIndex,
    projection: &mut SelectedRenderableProjection,
) {
    let Some(root) = scene_index.resolve(target) else {
        return;
    };
    if projection.target_roots.get(target) == Some(&root) {
        return;
    }
    projection.resolution_count = projection.resolution_count.saturating_add(1);
    projection.target_roots.insert(target.clone(), root);
    projection
        .roots_by_entity
        .entry(root)
        .or_default()
        .insert(target.clone());
    projection
        .target_renderables
        .entry(target.clone())
        .or_default();
    projection.pending_targets.push(PendingTargetProjection {
        target: target.clone(),
        stack: vec![root],
        visited: HashSet::new(),
    });
}

pub(super) fn reconcile_targets(
    targets: &[SceneAnchor],
    scene_index: &super::SceneAnchorIndex,
    projection: &mut SelectedRenderableProjection,
) -> bool {
    let desired = targets.iter().cloned().collect::<HashSet<_>>();
    projection
        .pending_targets
        .retain(|pending| desired.contains(&pending.target));

    let stale = projection
        .target_roots
        .keys()
        .filter(|target| !desired.contains(*target))
        .cloned()
        .collect::<Vec<_>>();
    let mut changed = false;
    for target in stale {
        changed |= remove_target_projection(&target, projection);
    }

    for target in targets {
        let root = scene_index.resolve(target);
        if projection.target_roots.get(target) != root.as_ref() {
            if projection.target_roots.contains_key(target) {
                remove_target_projection(target, projection);
            }
            if root.is_some() {
                schedule_target(target, scene_index, projection);
            }
            changed = true;
        }
    }
    changed
}

fn remove_target_projection(
    target: &SceneAnchor,
    projection: &mut SelectedRenderableProjection,
) -> bool {
    projection
        .pending_targets
        .retain(|pending| &pending.target != target);
    if let Some(root) = projection.target_roots.remove(target)
        && let Some(targets) = projection.roots_by_entity.get_mut(&root)
    {
        targets.remove(target);
        if targets.is_empty() {
            projection.roots_by_entity.remove(&root);
        }
    }
    let Some(renderables) = projection.target_renderables.remove(target) else {
        projection.target_bounds.remove(target);
        return false;
    };
    projection.pending_removals.push(PendingTargetRemoval {
        target: target.clone(),
        entities: renderables.into_iter(),
    });
    projection.target_bounds.remove(target);
    true
}

pub(super) fn advance_pending_removals(
    projection: &mut SelectedRenderableProjection,
    mut budget: usize,
) -> bool {
    let mut changed = false;
    while budget > 0 {
        let next = {
            let Some(work) = projection.pending_removals.last_mut() else {
                break;
            };
            let target = work.target.clone();
            work.entities.next().map(|entity| (target, entity))
        };
        let Some((target, entity)) = next else {
            projection.pending_removals.pop();
            continue;
        };
        budget -= 1;
        remove_renderable_membership(projection, &target, entity);
        changed = true;
    }
    changed
}

pub(super) fn advance_pending_targets(
    projection: &mut SelectedRenderableProjection,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
    geometry: &Query<(
        Option<&GlobalTransform>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
    bounds_requested: bool,
) -> (bool, bool) {
    let mut budget = super::MAX_PROJECTION_ENTITIES_PER_UPDATE;
    let mut mapping_changed = false;
    let mut bounds_changed = false;
    while budget > 0 {
        let Some(work) = projection.pending_targets.last_mut() else {
            break;
        };
        let Some(entity) = work.stack.pop() else {
            let target = work.target.clone();
            projection.pending_targets.pop();
            if bounds_requested {
                let bounds = projection
                    .target_renderables
                    .get(&target)
                    .and_then(|renderables| bounds_for_entities(renderables, geometry));
                replace_target_bounds(projection, &target, bounds);
                bounds_changed = true;
            }
            continue;
        };
        budget -= 1;
        let Some((target, mesh_present, children)) = (|| {
            if !work.visited.insert(entity) {
                return None;
            }
            let Ok((children, mesh)) = hierarchy.get(entity) else {
                return None;
            };
            Some((
                work.target.clone(),
                mesh.is_some(),
                children.map(|children| children.iter().collect::<Vec<_>>()),
            ))
        })() else {
            continue;
        };
        if mesh_present && add_target_renderable(projection, &target, entity) {
            mapping_changed = true;
        }
        if let Some(children) = children
            && let Some(work) = projection.pending_targets.last_mut()
        {
            work.stack.extend(children);
        }
    }
    (mapping_changed, bounds_changed)
}

pub(super) fn containing_targets(
    entity: Entity,
    roots_by_entity: &HashMap<Entity, HashSet<SceneAnchor>>,
    parents: &Query<Option<&ChildOf>>,
) -> HashSet<SceneAnchor> {
    let mut current = Some(entity);
    let mut visited = HashSet::new();
    let mut targets = HashSet::new();
    while let Some(entity) = current {
        if !visited.insert(entity) {
            break;
        }
        if let Some(roots) = roots_by_entity.get(&entity) {
            targets.extend(roots.iter().cloned());
        }
        current = parents.get(entity).ok().flatten().map(ChildOf::parent);
    }
    targets
}

pub(super) fn add_target_renderable(
    projection: &mut SelectedRenderableProjection,
    target: &SceneAnchor,
    entity: Entity,
) -> bool {
    let inserted = projection
        .target_renderables
        .entry(target.clone())
        .or_default()
        .insert(entity);
    if !inserted {
        return false;
    }
    projection
        .renderable_targets
        .entry(entity)
        .or_default()
        .insert(target.clone());
    let count = projection.renderable_refcounts.entry(entity).or_default();
    let was_unselected = *count == 0;
    *count += 1;
    projection.renderables.insert(entity);
    if was_unselected && !projection.removed_renderables.remove(&entity) {
        projection.added_renderables.insert(entity);
    }
    true
}

pub(super) fn remove_target_renderable(
    projection: &mut SelectedRenderableProjection,
    target: &SceneAnchor,
    entity: Entity,
) {
    let Some(renderables) = projection.target_renderables.get_mut(target) else {
        return;
    };
    if !renderables.remove(&entity) {
        return;
    }
    remove_renderable_membership(projection, target, entity);
}

fn remove_renderable_membership(
    projection: &mut SelectedRenderableProjection,
    target: &SceneAnchor,
    entity: Entity,
) {
    if let Some(targets) = projection.renderable_targets.get_mut(&entity) {
        targets.remove(target);
        if targets.is_empty() {
            projection.renderable_targets.remove(&entity);
        }
    }
    let Some(count) = projection.renderable_refcounts.get_mut(&entity) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        projection.renderable_refcounts.remove(&entity);
        projection.renderables.remove(&entity);
        if !projection.added_renderables.remove(&entity) {
            projection.removed_renderables.insert(entity);
        }
    }
}
