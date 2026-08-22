//! `UsdPrimRef` — the bridge between a Bevy entity and the USD prim path it
//! was projected from. The live editor's `SdfPath ↔ Entity` bimap keys off
//! it, and ECS-side code uses it to ask "what prim is this?".

use std::collections::HashMap;

use bevy::ecs::component::Component;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::{Entity, Resource};
use bevy::reflect::{Reflect, std_traits::ReflectDefault};
use usd_model::EntityKey;

/// The composed absolute prim path an entity was projected from
/// (e.g. `"/World/ChildA"`).
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq, Hash)]
#[reflect(Component, Default)]
pub struct UsdPrimRef {
    pub path: String,
}

impl UsdPrimRef {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Stable semantic identity attached to a projected Bevy entity.
///
/// This is intentionally separate from [`UsdPrimRef`]: a prim path identifies
/// the current location, while an `EntityKey` identifies the logical object
/// across path moves and revisions.
#[derive(Component, Clone, Debug, Eq, PartialEq)]
pub struct UsdEntityKey(pub EntityKey);

impl UsdEntityKey {
    pub fn new(key: EntityKey) -> Self {
        Self(key)
    }

    pub fn key(&self) -> &EntityKey {
        &self.0
    }
}

/// Bidirectional semantic-identity index for the current Bevy projection.
#[derive(Resource, Default)]
pub struct SemanticEntityIndex {
    by_key: HashMap<EntityKey, Entity>,
    by_entity: HashMap<Entity, EntityKey>,
}

impl SemanticEntityIndex {
    /// Insert or replace one semantic identity mapping while preserving the
    /// bimap invariant on both sides.
    pub fn insert(&mut self, key: EntityKey, entity: Entity) {
        if let Some(previous_entity) = self.by_key.insert(key.clone(), entity)
            && previous_entity != entity
        {
            self.by_entity.remove(&previous_entity);
        }
        if let Some(previous_key) = self.by_entity.insert(entity, key.clone())
            && previous_key != key
        {
            self.by_key.remove(&previous_key);
        }
    }

    pub fn entity(&self, key: &EntityKey) -> Option<Entity> {
        self.by_key.get(key).copied()
    }

    pub fn key(&self, entity: Entity) -> Option<&EntityKey> {
        self.by_entity.get(&entity)
    }

    pub fn remove_key(&mut self, key: &EntityKey) -> Option<Entity> {
        let entity = self.by_key.remove(key)?;
        self.by_entity.remove(&entity);
        Some(entity)
    }

    pub fn remove_entity(&mut self, entity: Entity) -> Option<EntityKey> {
        let key = self.by_entity.remove(&entity)?;
        self.by_key.remove(&key);
        Some(key)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

pub use crate::route::physics::UsdJoint;
pub use crate::route::skel::{UsdBlendShapeBinding, UsdSkelAnimDriver};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_index_replaces_both_sides_of_a_mapping() {
        let mut world = bevy::ecs::world::World::new();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let old_key = EntityKey::from("application:old");
        let new_key = EntityKey::from("application:new");
        let mut index = SemanticEntityIndex::default();

        index.insert(old_key.clone(), first);
        index.insert(new_key.clone(), first);
        assert_eq!(index.entity(&old_key), None);
        assert_eq!(index.entity(&new_key), Some(first));
        assert_eq!(index.key(first), Some(&new_key));

        index.insert(new_key.clone(), second);
        assert_eq!(index.entity(&new_key), Some(second));
        assert_eq!(index.key(first), None);
        assert_eq!(index.key(second), Some(&new_key));
    }
}
