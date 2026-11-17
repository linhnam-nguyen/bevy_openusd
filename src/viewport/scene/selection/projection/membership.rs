use std::collections::HashSet;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use viewport_protocol::SceneAnchor;

use super::{
    HierarchyCursor, PendingTargetProjection, PendingTargetRemoval, SelectedRenderableProjection,
};

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
    projection
        .target_renderable_order
        .entry(target.clone())
        .or_default();
    projection.pending_targets.push(PendingTargetProjection {
        target: target.clone(),
        stack: vec![HierarchyCursor::new(root)],
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
    projection
        .pending_bounds
        .retain(|pending| &pending.target != target);
    projection.target_renderable_order.remove(target);
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

pub(super) enum CursorStep {
    Entity {
        entity: Entity,
        mesh_present: bool,
    },
    /// One bounded cursor operation occurred without entering a new entity:
    /// duplicate-frame discard, child inspection/push, or exhausted-frame pop.
    Progress,
    Finished,
}

/// Advances exactly one hierarchy-cursor operation.
///
/// The caller owns the work budget. In particular, this function never loops
/// over a deep unwind: each push/pop/enter consumes one caller-visible step.
pub(super) fn advance_cursor(
    stack: &mut Vec<HierarchyCursor>,
    visited: &mut HashSet<Entity>,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
) -> CursorStep {
    let Some(cursor) = stack.last() else {
        return CursorStep::Finished;
    };
    let (entity, entered, next_child) = (cursor.entity, cursor.entered, cursor.next_child);

    if !entered {
        if let Some(cursor) = stack.last_mut() {
            cursor.entered = true;
        }
        if !visited.insert(entity) {
            stack.pop();
            return CursorStep::Progress;
        }
        let mesh_present = hierarchy.get(entity).is_ok_and(|(_, mesh)| mesh.is_some());
        return CursorStep::Entity {
            entity,
            mesh_present,
        };
    }

    let next = hierarchy
        .get(entity)
        .ok()
        .and_then(|(children, _)| children.and_then(|children| children.get(next_child)))
        .copied();
    if let Some(child) = next {
        if let Some(cursor) = stack.last_mut() {
            cursor.next_child += 1;
        }
        stack.push(HierarchyCursor::new(child));
    } else {
        stack.pop();
    }
    CursorStep::Progress
}

pub(super) fn advance_pending_targets(
    projection: &mut SelectedRenderableProjection,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
) -> (bool, Vec<SceneAnchor>, usize) {
    let mut budget = super::MAX_PROJECTION_ENTITIES_PER_UPDATE;
    let mut work_done = 0;
    let mut mapping_changed = false;
    let mut completed_targets = Vec::new();

    while budget > 0 {
        let step = {
            let Some(work) = projection.pending_targets.last_mut() else {
                break;
            };
            let target = work.target.clone();
            (
                target,
                advance_cursor(&mut work.stack, &mut work.visited, hierarchy),
            )
        };

        let (target, step) = step;
        match step {
            CursorStep::Finished => {
                projection.pending_targets.pop();
                completed_targets.push(target);
            }
            CursorStep::Progress => {
                budget -= 1;
                work_done += 1;
            }
            CursorStep::Entity {
                entity,
                mesh_present,
            } => {
                budget -= 1;
                work_done += 1;
                if mesh_present && add_target_renderable(projection, &target, entity) {
                    mapping_changed = true;
                }
            }
        }
    }

    (mapping_changed, completed_targets, work_done)
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
    let ordered = projection
        .target_renderable_order
        .entry(target.clone())
        .or_default()
        .insert(entity);
    debug_assert!(
        ordered,
        "new target membership must have exactly one order entry"
    );
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
) -> bool {
    let Some(renderables) = projection.target_renderables.get_mut(target) else {
        return false;
    };
    if !renderables.remove(&entity) {
        return false;
    }
    if let Some(order) = projection.target_renderable_order.get_mut(target) {
        let ordered = order.remove(entity);
        debug_assert!(
            ordered,
            "removed target membership must have exactly one order entry"
        );
    }
    // Dense swap-removal changes order indices. Restart only this target's
    // already-bounded bounds pass; the topology caller will schedule it again.
    projection
        .pending_bounds
        .retain(|pending| &pending.target != target);
    remove_renderable_membership(projection, target, entity);
    true
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
