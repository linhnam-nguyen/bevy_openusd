//! Test-only helpers attached to the owning production module.
use super::*;

impl SelectedRenderableProjection {
    pub(crate) fn is_pending(&self) -> bool {
        !self.pending_targets.is_empty()
            || !self.pending_removals.is_empty()
            || !self.pending_bounds.is_empty()
            || self.pending_topology.is_some()
            || !self.topology_queue.is_empty()
    }

    pub(crate) fn pending_hierarchy_depth(&self) -> usize {
        self.pending_targets
            .iter()
            .map(|work| work.stack.len())
            .sum()
    }

    pub(crate) fn pending_topology_count(&self) -> usize {
        self.topology_queue.len() + usize::from(self.pending_topology.is_some())
    }

    pub(crate) fn last_cursor_work(&self) -> usize {
        self.last_cursor_work
    }

    pub(crate) fn last_topology_work(&self) -> usize {
        self.last_topology_work
    }

    pub(crate) fn target_order_len(&self, target: &SceneAnchor) -> usize {
        self.target_renderable_order
            .get(target)
            .map_or(0, EntityOrder::len)
    }
}
