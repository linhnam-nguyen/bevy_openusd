use std::collections::HashMap;

use bevy::prelude::Entity;

/// Dense live-entity order with O(1) membership, insertion, and removal.
///
/// Unlike an append-only `Vec<Entity>`, this structure never retains historical
/// tombstones. Removing and later re-adding an entity produces exactly one live
/// entry, so memory stays O(current membership).
#[derive(Clone, Debug, Default)]
pub(super) struct EntityOrder {
    entities: Vec<Entity>,
    index_by_entity: HashMap<Entity, usize>,
}

impl EntityOrder {
    pub(super) fn insert(&mut self, entity: Entity) -> bool {
        if self.index_by_entity.contains_key(&entity) {
            return false;
        }
        let index = self.entities.len();
        self.entities.push(entity);
        self.index_by_entity.insert(entity, index);
        true
    }

    pub(super) fn remove(&mut self, entity: Entity) -> bool {
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

    pub(super) fn clear(&mut self) {
        self.entities.clear();
        self.index_by_entity.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.entities.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub(super) fn get(&self, index: usize) -> Option<Entity> {
        self.entities.get(index).copied()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::World;

    #[test]
    fn remove_readd_churn_never_grows_history() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let mut order = EntityOrder::default();

        for _ in 0..10_000 {
            assert!(order.insert(entity));
            assert!(!order.insert(entity));
            assert_eq!(order.len(), 1);
            assert!(order.remove(entity));
            assert_eq!(order.len(), 0);
        }

        assert!(order.insert(entity));
        assert_eq!(order.iter().collect::<Vec<_>>(), vec![entity]);
    }

    #[test]
    fn swap_remove_repairs_the_moved_index() {
        let mut world = World::new();
        let first = world.spawn_empty().id();
        let middle = world.spawn_empty().id();
        let last = world.spawn_empty().id();
        let mut order = EntityOrder::default();

        assert!(order.insert(first));
        assert!(order.insert(middle));
        assert!(order.insert(last));
        assert!(order.remove(middle));
        assert_eq!(order.len(), 2);
        assert!(order.remove(last));
        assert_eq!(order.iter().collect::<Vec<_>>(), vec![first]);
    }
}
