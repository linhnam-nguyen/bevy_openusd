use bevy::prelude::Entity;
use viewport_protocol::{HierarchyVisibilityState, SceneAnchor};

use super::SceneAnchorIndex;

impl SceneAnchorIndex {
    pub(crate) fn resolve(&self, anchor: &SceneAnchor) -> Option<Entity> {
        self.dense
            .by_anchor
            .get(anchor)
            .and_then(|index| self.dense.node(*index))
            .and_then(|node| node.entity)
            .or_else(|| self.by_anchor.get(anchor).copied())
    }

    /// Resolves every current scene occurrence for a semantic prim path.
    /// Semantic classification entries intentionally carry path identity only;
    /// native-instance projection may expose the same path under multiple
    /// scene-local instance contexts.
    pub(crate) fn resolve_all_by_prim_path(&self, prim_path: &str) -> &[Entity] {
        self.occurrence_index.resolve(prim_path)
    }

    pub(crate) fn visibility_for_anchor(&self, anchor: &SceneAnchor) -> HierarchyVisibilityState {
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
            return HierarchyVisibilityState::Visible;
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
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}
