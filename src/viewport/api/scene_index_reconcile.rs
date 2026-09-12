//! Resumable authority for whole-index reconciliation.
//!
//! ECS data is copied in bounded chunks on the Bevy update thread. Sorting,
//! occurrence assignment, dense-index construction, and hierarchy projection
//! are then built from owned rows on a worker thread and atomically published.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};

use super::SceneAnchorIndex;
use super::rebuild::{SceneIndexCandidate, SceneIndexSnapshot, build_snapshot};
use crate::viewport::session::StagePresentationContext;

type WorkerResult = Result<SceneIndexSnapshot, ()>;

struct RunningRebuild {
    epoch: u64,
    slot: Arc<Mutex<Option<WorkerResult>>>,
}

#[derive(Default)]
pub(super) struct SceneIndexReconcileState {
    members: Vec<Entity>,
    positions: HashMap<Entity, usize>,
    requested_epoch: u64,
    settled_epoch: u64,
    capture_epoch: Option<u64>,
    capture_offset: usize,
    capture_chunks: Vec<Vec<SceneIndexCandidate>>,
    running: Option<RunningRebuild>,
    last_capture_work: usize,
}

impl SceneIndexReconcileState {
    pub(super) fn admit(&mut self, entity: Entity) -> bool {
        if self.positions.contains_key(&entity) {
            return false;
        }
        let index = self.members.len();
        self.members.push(entity);
        self.positions.insert(entity, index);
        true
    }

    pub(super) fn remove(&mut self, entity: Entity) -> bool {
        let Some(index) = self.positions.remove(&entity) else {
            return false;
        };
        let last = self.members.len() - 1;
        self.members.swap_remove(index);
        if index != last {
            let moved = self.members[index];
            self.positions.insert(moved, index);
        }
        self.request();
        true
    }

    pub(super) fn contains(&self, entity: Entity) -> bool {
        self.positions.contains_key(&entity)
    }

    pub(super) fn request(&mut self) {
        self.requested_epoch = self.requested_epoch.saturating_add(1);
        if self.requested_epoch == self.settled_epoch {
            self.requested_epoch = self.requested_epoch.saturating_add(1);
        }
    }

    pub(super) fn is_pending(&self) -> bool {
        self.requested_epoch != self.settled_epoch
            || self.capture_epoch.is_some()
            || self.running.is_some()
    }

    pub(super) fn member_count(&self) -> usize {
        self.members.len()
    }

    pub(super) fn last_capture_work(&self) -> usize {
        self.last_capture_work
    }
}

impl SceneAnchorIndex {
    pub(super) fn advance_full_reconciliation(
        &mut self,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&UsdHierarchyTarget>,
            Option<&UsdTransparentHierarchyNode>,
            Option<&Visibility>,
            Option<&Children>,
        )>,
        parents: &Query<Option<&ChildOf>>,
        presentation: Option<&StagePresentationContext>,
        budget: usize,
    ) -> Option<super::super::hierarchy::CurrentHierarchyProjection> {
        self.reconcile.last_capture_work = 0;

        let ready = self.reconcile.running.as_ref().and_then(|running| {
            let mut slot = running
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.take().map(|result| (running.epoch, result))
        });
        if let Some((epoch, result)) = ready {
            self.reconcile.running = None;
            if epoch == self.reconcile.requested_epoch {
                match result {
                    Ok(snapshot) => {
                        let projection = self.install_reconciled_snapshot(snapshot);
                        self.reconcile.settled_epoch = epoch;
                        self.reconcile.capture_epoch = None;
                        self.reconcile.capture_offset = 0;
                        self.reconcile.capture_chunks.clear();
                        return Some(projection);
                    }
                    Err(()) => {
                        // A worker panic never leaves the resource permanently
                        // pending on an unreachable result. Re-capture the same
                        // authoritative epoch in bounded chunks.
                        self.reconcile.capture_epoch = None;
                        self.reconcile.capture_offset = 0;
                        self.reconcile.capture_chunks.clear();
                    }
                }
            } else {
                // A newer mutation superseded this immutable candidate. Drop
                // it and restart from the current membership/order.
                self.reconcile.capture_epoch = None;
                self.reconcile.capture_offset = 0;
                self.reconcile.capture_chunks.clear();
            }
        } else if self.reconcile.running.is_some() {
            return None;
        }

        if self.reconcile.requested_epoch == self.reconcile.settled_epoch {
            return None;
        }
        let epoch = self.reconcile.requested_epoch;
        if self.reconcile.capture_epoch != Some(epoch) {
            self.reconcile.capture_epoch = Some(epoch);
            self.reconcile.capture_offset = 0;
            self.reconcile.capture_chunks.clear();
        }

        let remaining = self
            .reconcile
            .members
            .len()
            .saturating_sub(self.reconcile.capture_offset);
        let mut chunk = Vec::with_capacity(remaining.min(budget));
        while self.reconcile.last_capture_work < budget
            && self.reconcile.capture_offset < self.reconcile.members.len()
        {
            let entity = self.reconcile.members[self.reconcile.capture_offset];
            self.reconcile.capture_offset += 1;
            self.reconcile.last_capture_work += 1;
            if let Some(candidate) = capture_candidate(entity, prims, parents, presentation) {
                chunk.push(candidate);
            }
        }
        if !chunk.is_empty() {
            self.reconcile.capture_chunks.push(chunk);
        }
        if self.reconcile.capture_offset < self.reconcile.members.len() {
            return None;
        }

        let chunks = std::mem::take(&mut self.reconcile.capture_chunks);
        self.reconcile.capture_epoch = None;
        self.reconcile.capture_offset = 0;
        let revision = self.revision.saturating_add(1);
        let slot = Arc::new(Mutex::new(None));
        let worker_slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build_snapshot(chunks, revision)
            }))
            .map_err(|_| ());
            let mut slot = worker_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *slot = Some(result);
        });
        self.reconcile.running = Some(RunningRebuild { epoch, slot });
        None
    }

    fn install_reconciled_snapshot(
        &mut self,
        snapshot: SceneIndexSnapshot,
    ) -> super::super::hierarchy::CurrentHierarchyProjection {
        let SceneIndexSnapshot {
            by_anchor,
            by_entity,
            occurrence_index,
            nodes,
            node_index_by_anchor,
            dense,
            parent_by_entity,
            transparent_by_entity,
            projection,
            revision,
        } = snapshot;
        let row_count = nodes.len() as u64;

        self.by_anchor = by_anchor;
        self.by_entity = by_entity;
        self.occurrence_index = occurrence_index;
        self.nodes = nodes;
        self.node_index_by_anchor = node_index_by_anchor;
        self.dense = dense;
        self.parent_by_entity = parent_by_entity;
        self.transparent_by_entity = transparent_by_entity;
        self.revision = revision;
        self.initialized = true;
        self.derived_dirty = false;
        self.rebuild_count = self.rebuild_count.saturating_add(1);
        self.incremental_work.reindexed_rows = self
            .incremental_work
            .reindexed_rows
            .saturating_add(row_count);
        self.incremental_work.projected_rows = self
            .incremental_work
            .projected_rows
            .saturating_add(row_count);
        projection
    }
}

pub(super) fn capture_candidate(
    entity: Entity,
    prims: &Query<(
        Entity,
        &UsdPrimRef,
        Option<&UsdDisplayName>,
        Option<&UsdHierarchyTarget>,
        Option<&UsdTransparentHierarchyNode>,
        Option<&Visibility>,
        Option<&Children>,
    )>,
    parents: &Query<Option<&ChildOf>>,
    presentation: Option<&StagePresentationContext>,
) -> Option<SceneIndexCandidate> {
    let (_, prim, authored, target, transparent, visibility, _) = prims.get(entity).ok()?;
    if prim.path == "/" {
        return None;
    }
    Some(SceneIndexCandidate {
        entity,
        path: prim.path.clone(),
        name: super::prim_name(&prim.path).to_owned(),
        display_name: super::incremental::display_name(prim, authored, target, presentation),
        transparent: transparent.is_some(),
        visible: !matches!(visibility, Some(Visibility::Hidden)),
        parent: parents.get(entity).ok().flatten().map(ChildOf::parent),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_member_removal_repairs_the_swapped_position() {
        let a = Entity::from_bits(1);
        let b = Entity::from_bits(2);
        let c = Entity::from_bits(3);
        let mut state = SceneIndexReconcileState::default();
        assert!(state.admit(a));
        assert!(state.admit(b));
        assert!(state.admit(c));

        assert!(state.remove(a));
        assert!(!state.contains(a));
        assert!(state.contains(b));
        assert!(state.contains(c));
        assert_eq!(state.members.len(), 2);
        for (index, entity) in state.members.iter().copied().enumerate() {
            assert_eq!(state.positions.get(&entity), Some(&index));
        }
    }
}
