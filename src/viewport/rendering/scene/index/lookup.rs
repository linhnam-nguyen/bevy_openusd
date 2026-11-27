use bevy::prelude::Entity;
use viewport_protocol::{HierarchyVisibilityState, SceneAnchor};

use super::SceneAnchorIndex;

impl SceneAnchorIndex {
    pub(crate) fn point_presentation_key(&self, anchor: &SceneAnchor) -> Option<(i64, u64)> {
        if !self.accepts_logical_prim_path(&anchor.prim_path) {
            return None;
        }
        self.point_occurrences.presentation_key(anchor)
    }

    pub(crate) fn is_point_target(&self, anchor: &SceneAnchor) -> bool {
        self.accepts_logical_prim_path(&anchor.prim_path)
            && self.point_occurrences.resolve(anchor).is_some()
    }

    pub(crate) fn point_anchor(&self, instancer: &str, logical_id: i64) -> Option<SceneAnchor> {
        if !self.accepts_logical_prim_path(instancer) {
            return None;
        }
        self.point_occurrences
            .anchor_for_logical_id(instancer, logical_id)
    }

    pub(crate) fn resolve(&self, anchor: &SceneAnchor) -> Option<Entity> {
        if self.has_logical_authority() && (!self.logical_hierarchy_available()
            || (!self.logical_occurrences.occurrences.contains_key(anchor)
                && !self.is_point_target(anchor)))
        {
            return None;
        }
        self.dense
            .by_anchor
            .get(anchor)
            .and_then(|index| self.dense.node(*index))
            .and_then(|node| node.entity)
            .or_else(|| self.by_anchor.get(anchor).copied())
            .or_else(|| self.point_occurrences.resolve(anchor))
    }

    /// Resolves every current scene occurrence for a semantic prim path.
    /// Semantic classification entries intentionally carry path identity only;
    /// native-instance projection may expose the same path under multiple
    /// scene-local instance contexts.
    pub(crate) fn resolve_all_by_prim_path(&self, prim_path: &str) -> &[Entity] {
        if !self.accepts_logical_prim_path(prim_path) {
            return &[];
        }
        self.occurrence_index.resolve(prim_path)
    }

    pub(crate) fn visibility_for_anchor(&self, anchor: &SceneAnchor) -> HierarchyVisibilityState {
        if let Some(visible) = self.logical_visibility(anchor) {
            return HierarchyVisibilityState::from_visible(visible);
        }
        if self.is_point_target(anchor)
            && let Some(visibility) = self.point_occurrences.visibility(anchor)
        {
            return visibility;
        }
        self.dense
            .by_anchor
            .get(anchor)
            .and_then(|index| self.dense.node(*index))
            .map_or(HierarchyVisibilityState::Visible, |node| {
                HierarchyVisibilityState::from_visible(node.visible)
            })
    }

    pub(crate) fn visibility_for_prim_path(&self, prim_path: &str) -> HierarchyVisibilityState {
        let mut states = self
            .dense
            .by_path
            .get(prim_path)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(|index| self.dense.node(*index))
            .map(|node| HierarchyVisibilityState::from_visible(node.visible));
        let Some(first) = states.next() else {
            return self.logical_visibility(&SceneAnchor::active_session(prim_path))
                .map_or(HierarchyVisibilityState::Visible, HierarchyVisibilityState::from_visible);
        };
        if states.all(|state| state == first) {
            first
        } else {
            HierarchyVisibilityState::Mixed
        }
    }

    pub(crate) fn anchor_for(&self, entity: Entity) -> Option<SceneAnchor> {
        self.dense
            .by_entity
            .get(&entity)
            .and_then(|index| self.dense.node(*index))
            .map(|node| node.anchor.clone())
            .or_else(|| self.by_entity.get(&entity).cloned())
            .or_else(|| self.point_occurrences.anchor_for(entity))
            .filter(|anchor| !self.has_logical_authority()
                || (self.logical_hierarchy_available()
                    && (self.logical_occurrences.occurrences.contains_key(anchor)
                        || self.is_point_target(anchor))))
    }

    pub(crate) fn revision(&self) -> u64 {
        self.published_revision
            .saturating_add(self.point_occurrences.revision)
    }

    pub(crate) fn has_pending_initial_publication(&self, activation_generation: u64) -> bool {
        self.published_activation_generation != Some(activation_generation)
            && (self.derived_dirty
                || !self.pending_additions.is_empty()
                || self.reconcile.is_pending())
    }
}
