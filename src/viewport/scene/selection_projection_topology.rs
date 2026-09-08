//! Bounded sparse topology reconciliation for logical selection projection.
//!
//! Both descendant cursor work and selected-ancestor resolution are resumable.
//! Every hierarchy push/pop/enter and every parent hop is charged to the same
//! caller-owned update budget.

use std::collections::HashSet;

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use viewport_protocol::SceneAnchor;

use super::membership::{
    CursorStep, add_target_renderable, advance_cursor, remove_target_renderable,
};
use super::{HierarchyCursor, SelectedRenderableProjection};

#[derive(Debug)]
struct PendingTopologyEntity {
    entity: Entity,
    ancestor: Option<Entity>,
    visited_ancestors: HashSet<Entity>,
    current_targets: HashSet<SceneAnchor>,
}

#[derive(Debug)]
pub(super) struct PendingTopologyProjection {
    pub(super) recurse_descendants: bool,
    pub(super) processed_single: bool,
    pub(super) stack: Vec<HierarchyCursor>,
    pub(super) visited: HashSet<Entity>,
    pending_entity: Option<PendingTopologyEntity>,
}

pub(super) struct TopologyProgress {
    pub(super) mapping_changed: bool,
    pub(super) changed_targets: HashSet<SceneAnchor>,
    pub(super) work: usize,
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
        work: 0,
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
                pending_entity: None,
            });
        }

        let advanced_ancestor = {
            let work = projection
                .pending_topology
                .as_mut()
                .expect("topology work was initialized above");
            match work.pending_entity.as_mut() {
                Some(pending) => match pending.ancestor {
                    Some(ancestor) => {
                        if !pending.visited_ancestors.insert(ancestor) {
                            pending.ancestor = None;
                        } else {
                            if let Some(roots) = projection.roots_by_entity.get(&ancestor) {
                                pending.current_targets.extend(roots.iter().cloned());
                            }
                            pending.ancestor =
                                parents.get(ancestor).ok().flatten().map(ChildOf::parent);
                        }
                        true
                    }
                    None => false,
                },
                None => false,
            }
        };
        if advanced_ancestor {
            budget -= 1;
            progress.work += 1;
            continue;
        }

        let completed_entity = {
            let work = projection
                .pending_topology
                .as_mut()
                .expect("topology work was initialized above");
            if work
                .pending_entity
                .as_ref()
                .is_some_and(|pending| pending.ancestor.is_none())
            {
                work.pending_entity.take()
            } else {
                None
            }
        };
        if let Some(completed) = completed_entity {
            budget -= 1;
            progress.work += 1;
            reconcile_entity_targets(projection, completed, &mut progress);
            continue;
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

        match step {
            CursorStep::Progress => {
                budget -= 1;
                progress.work += 1;
            }
            CursorStep::Finished => {
                projection.pending_topology = None;
            }
            CursorStep::Entity {
                entity,
                mesh_present,
            } => {
                budget -= 1;
                progress.work += 1;
                projection
                    .pending_topology
                    .as_mut()
                    .expect("topology work was initialized above")
                    .pending_entity = Some(PendingTopologyEntity {
                    entity,
                    ancestor: mesh_present.then_some(entity),
                    visited_ancestors: HashSet::new(),
                    current_targets: HashSet::new(),
                });
            }
        }
    }

    progress
}

fn reconcile_entity_targets(
    projection: &mut SelectedRenderableProjection,
    completed: PendingTopologyEntity,
    progress: &mut TopologyProgress,
) {
    let previous_targets = projection
        .renderable_targets
        .get(&completed.entity)
        .cloned()
        .unwrap_or_default();

    for target in previous_targets.difference(&completed.current_targets) {
        if remove_target_renderable(projection, target, completed.entity) {
            progress.mapping_changed = true;
            progress.changed_targets.insert(target.clone());
        }
    }
    for target in completed.current_targets.difference(&previous_targets) {
        if add_target_renderable(projection, target, completed.entity) {
            progress.mapping_changed = true;
            progress.changed_targets.insert(target.clone());
        }
    }
}
