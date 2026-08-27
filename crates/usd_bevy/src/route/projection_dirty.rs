use std::collections::{HashMap, HashSet};

use bevy::asset::AssetId;
use bevy::mesh::Mesh;
use bevy::prelude::{Entity, Resource};

/// Entities whose render projection changed during the authoritative USD
/// routing pass. Consumers drain this set instead of polling ECS change ticks.
#[derive(Resource, Default)]
pub struct RenderProjectionDirtySet {
    entities: HashSet<Entity>,
}

impl RenderProjectionDirtySet {
    pub fn mark(&mut self, entity: Entity) {
        self.entities.insert(entity);
    }

    pub(crate) fn remove(&mut self, entity: Entity) {
        self.entities.remove(&entity);
    }

    pub fn take(&mut self) -> Vec<Entity> {
        self.entities.drain().collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entities.len()
    }
}

/// Reverse index from projected mesh assets to the USD entities consuming them.
/// Asset events can therefore dirty only affected render entities.
#[derive(Resource, Default)]
pub struct MeshProjectionConsumers {
    by_mesh: HashMap<AssetId<Mesh>, HashSet<Entity>>,
    by_entity: HashMap<Entity, AssetId<Mesh>>,
}

impl MeshProjectionConsumers {
    /// Register the current mesh handle for one projected render entity.
    pub fn track(&mut self, entity: Entity, mesh: AssetId<Mesh>) -> bool {
        if self.by_entity.get(&entity).copied() == Some(mesh) {
            return false;
        }
        self.remove(entity);
        self.by_entity.insert(entity, mesh);
        self.by_mesh.entry(mesh).or_default().insert(entity);
        true
    }

    pub(crate) fn remove(&mut self, entity: Entity) {
        let Some(mesh) = self.by_entity.remove(&entity) else {
            return;
        };
        let Some(consumers) = self.by_mesh.get_mut(&mesh) else {
            return;
        };
        consumers.remove(&entity);
        if consumers.is_empty() {
            self.by_mesh.remove(&mesh);
        }
    }

    pub fn consumers_for(&self, mesh: AssetId<Mesh>) -> Vec<Entity> {
        self.by_mesh
            .get(&mesh)
            .map(|entities| entities.iter().copied().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn consumer_count(&self, mesh: AssetId<Mesh>) -> usize {
        self.by_mesh.get(&mesh).map_or(0, HashSet::len)
    }
}

#[cfg(test)]
#[path = "projection_dirty_tests.rs"]
mod tests;
