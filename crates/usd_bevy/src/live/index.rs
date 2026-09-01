use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::path::{PathId, PathStore};

/// Bidirectional `PathId ↔ Entity` index — the reprojection key.
///
/// Path bytes are owned only by [`PathStore`]. This index stores compact IDs
/// and a parent-to-children topology for subtree traversal.
#[derive(Resource, Default)]
pub struct PrimEntities {
    by_path: HashMap<PathId, Entity>,
    by_entity: HashMap<Entity, PathId>,
    by_parent: HashMap<PathId, HashSet<PathId>>,
}

impl PrimEntities {
    pub fn insert(&mut self, paths: &mut PathStore, path: impl AsRef<str>, entity: Entity) {
        let path = paths.intern(path);
        if let Some(previous_path) = self.by_entity.get(&entity).copied()
            && previous_path != path
        {
            self.remove_id(paths, previous_path);
        }
        if let Some(previous_entity) = self.by_path.get(&path).copied()
            && previous_entity != entity
        {
            self.by_entity.remove(&previous_entity);
        }
        self.by_entity.insert(entity, path);
        self.by_path.insert(path, entity);
        if let Some(parent) = paths.parent(path) {
            self.by_parent.entry(parent).or_default().insert(path);
        }
    }

    pub fn entity(&self, paths: &PathStore, path: &str) -> Option<Entity> {
        self.id(paths, path).and_then(|id| self.entity_id(id))
    }

    pub fn id(&self, paths: &PathStore, path: &str) -> Option<PathId> {
        let id = paths.lookup(path)?;
        self.by_path.contains_key(&id).then_some(id)
    }

    pub fn entity_id(&self, path: PathId) -> Option<Entity> {
        self.by_path.get(&path).copied()
    }

    pub fn path<'a>(&self, paths: &'a PathStore, entity: Entity) -> Option<&'a str> {
        self.by_entity.get(&entity).and_then(|id| paths.path(*id))
    }

    /// Remove a path's mapping, returning the entity it pointed at.
    pub fn remove_path(&mut self, paths: &PathStore, path: &str) -> Option<Entity> {
        let id = self.id(paths, path)?;
        self.remove_id(paths, id)
    }

    /// Remove an entity's mapping (e.g. on despawn).
    pub fn remove_entity(&mut self, paths: &PathStore, entity: Entity) -> Option<String> {
        let id = self.by_entity.get(&entity).copied()?;
        self.remove_id(paths, id);
        paths.path(id).map(str::to_owned)
    }

    /// Every `(path, entity)` currently mapped.
    pub fn iter<'a>(
        &'a self,
        paths: &'a PathStore,
    ) -> impl Iterator<Item = (&'a str, Entity)> + 'a {
        self.by_path
            .iter()
            .filter_map(|(id, entity)| paths.path(*id).map(|path| (path, *entity)))
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Every `(path, entity)` whose path is `prefix` or a descendant of it —
    /// the set a `resynced` parent invalidates.
    pub fn subtree(&self, paths: &PathStore, prefix: &str) -> Vec<(PathId, Entity)> {
        let Some(root) = paths.lookup(prefix) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        let mut pending = vec![root];
        while let Some(path) = pending.pop() {
            if let Some(entity) = self.by_path.get(&path).copied() {
                ids.push((path, entity));
            }
            if let Some(children) = self.by_parent.get(&path) {
                pending.extend(children.iter().copied());
            }
        }
        ids.sort_unstable_by(|(left, _), (right, _)| paths.path(*left).cmp(&paths.path(*right)));
        ids
    }

    pub(crate) fn remove_id(&mut self, paths: &PathStore, path: PathId) -> Option<Entity> {
        let entity = self.by_path.remove(&path)?;
        self.by_entity.remove(&entity);
        if let Some(parent) = paths.parent(path) {
            let remove_parent = self.by_parent.get_mut(&parent).is_some_and(|children| {
                children.remove(&path);
                children.is_empty()
            });
            if remove_parent {
                self.by_parent.remove(&parent);
            }
        }
        Some(entity)
    }

    pub(crate) fn clear(&mut self) {
        self.by_path.clear();
        self.by_entity.clear();
        self.by_parent.clear();
    }
}
