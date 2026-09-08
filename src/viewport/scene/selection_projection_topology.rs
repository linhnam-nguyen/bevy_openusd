//! Bounded sparse topology reconciliation for logical selection projection.
//!
//! Split from selection_projection_membership.rs so both responsibilities stay
//! below the repository source-size hard limit.

use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use viewport_protocol::SceneAnchor;

use super::membership::{
    CursorStep, add_target_renderable, advance_cursor, remove_target_renderable,
};
use super::{HierarchyCursor, PendingTopologyProjection, SelectedRenderableProjection};

pub(super) struct TopologyProgress {
    pub(super) mapping_changed: bool,
    pub(super) changed_targets: HashSet<SceneAnchor>,
}

pub(super) fn advance_pending_topology(
    projection: &mut SelectedRenderableProjection,
    hierarchy: &Query<(Option<&Children>, Option<&Mesh3d>)>,
    parents: &Query<Option<&ChildOf>>,
    mut budget: usize,
) -> TopologyProgress {
    let mut progress = TopologyProgress {
        mapping_changed: false,
        changed_targets: HashSet::new(),
    };
    while budget > 0 {
        if projection.pending_topology.is_none() {
            let Some(root) = projection.topology_queue.pop_front() else {
                break;
            };
            let recurse_descendants = projection.queued_topology.remove(&root).unwrap_or(false);
            projection.pending_topology = Some(PendingTopologyProjection {
                recurse_descendants,
                processed_single: false,
                stack: vec![HierarchyCursor::new(root)],
                visited: HashSet::new(),
            });
        }

        let step = {
            let work = projection
                .pending_topology
                .as_mut()
                .expect("topology work was initialized above");
            if !work.recurse_descendants {
                if work.processed_single {
                    CursorStep::Finished
                } else {
                    work.processed_single = true;
                    CursorStep::Entity {
                        entity: work.stack[0].entity,
                        mesh_present: hierarchy
                            .get(work.stack[0].entity)
                            .is_ok_and(|(_, mesh)| mesh.is_some()),
                    }
                }
            } else {
                advance_cursor(&mut work.stack, &mut work.visited, hierarchy)
            }
        };

        let CursorStep::Entity {
            entity,
            mesh_present,
        } = step
        else {
            projection.pending_topology = None;
            continue;
        };
        budget -= 1;
        let current_targets = mesh_present
            .then(|| containing_targets(entity, &projection.roots_by_entity, parents))
            .unwrap_or_default();
        let previous_targets = projection
            .renderable_targets
            .get(&entity)
            .cloned()
            .unwrap_or_default();
        for target in previous_targets.difference(&current_targets) {
            if remove_target_renderable(projection, target, entity) {
                progress.mapping_changed = true;
                progress.changed_targets.insert(target.clone());
            }
        }
        for target in current_targets.difference(&previous_targets) {
            if add_target_renderable(projection, target, entity) {
                progress.mapping_changed = true;
                progress.changed_targets.insert(target.clone());
            }
        }
    }
    progress
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
