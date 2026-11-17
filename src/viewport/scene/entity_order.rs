use std::collections::HashMap;

use bevy::prelude::Entity;

/// Dense live-entity order with O(1) membership, insertion, and removal.
///
/// Unlike an append-only `Vec<Entity>`, this structure never retains historical
/// tombstones. Removing and later re-adding an entity produces exactly one live
/// entry, so memory stays O(current membership).
#[derive(Clone, Debug, Default)]
pub(in crate::viewport) struct EntityOrder {
    entities: Vec<Entity>,
    index_by_entity: HashMap<Entity, usize>,
}

impl EntityOrder {
    pub(in crate::viewport) fn insert(&mut self, entity: Entity) -> bool {
        if self.index_by_entity.contains_key(&entity) {
            return false;
        }
        let index = self.entities.len();
        self.entities.push(entity);
        self.index_by_entity.insert(entity, index);
        true
    }

    pub(in crate::viewport) fn remove(&mut self, entity: Entity) -> bool {
        let Some(index) = self.index_by_entity.remove(&entity) else {
            return false;
        };
        let removed = self.entities.swap_remove(index);
        debug_assert_eq!(removed, entity);
        if index < self.entities.len() {
            let moved = self.entities[index];
            let moved_index = self
                .index_by_entity
                .get_mut(&moved)
                .expect("dense order index exists for moved entity");
            *moved_index = index;
        }
        true
    }

    pub(in crate::viewport) fn clear(&mut self) {
        self.entities.clear();
        self.index_by_entity.clear();
    }

    pub(in crate::viewport) fn len(&self) -> usize {
        self.entities.len()
    }

    pub(in crate::viewport) fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub(in crate::viewport) fn get(&self, index: usize) -> Option<Entity> {
        self.entities.get(index).copied()
    }

    pub(in crate::viewport) fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter().copied()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/entity_order_inline_tests.rs"]
mod tests;
