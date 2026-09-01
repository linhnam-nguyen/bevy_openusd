use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::path::{is_descendant_or_self, normalize_prim_path};

/// Bidirectional `SdfPath ↔ Entity` index — the reprojection key. Plain
/// `Resource` (the paths are owned `String`s, the entities are ids).
#[derive(Resource, Default)]
pub struct PrimEntities {
    by_path: HashMap<String, Entity>,
    by_entity: HashMap<Entity, String>,
    /// Prefix-to-path postings used by subtree reconciliation. This keeps a
    /// small resync local instead of scanning every projected prim.
    by_prefix: HashMap<String, HashSet<String>>,
}

impl PrimEntities {
    pub fn insert(&mut self, path: impl Into<String>, entity: Entity) {
        let path = path.into();
        if let Some(previous_path) = self.by_entity.get(&entity).cloned()
            && previous_path != path
        {
            self.remove_path(&previous_path);
        }
        if let Some(previous_entity) = self.by_path.get(&path).copied()
            && previous_entity != entity
        {
            self.by_entity.remove(&previous_entity);
        }
        self.by_entity.insert(entity, path.clone());
        self.by_path.insert(path, entity);
        let path = self
            .by_entity
            .get(&entity)
            .expect("entity mapping inserted");
        for prefix in prefixes(path) {
            self.by_prefix
                .entry(prefix)
                .or_default()
                .insert(path.clone());
        }
    }

    pub fn entity(&self, path: &str) -> Option<Entity> {
        self.by_path.get(path).copied()
    }

    pub fn path(&self, entity: Entity) -> Option<&str> {
        self.by_entity.get(&entity).map(String::as_str)
    }

    /// Remove a path's mapping, returning the entity it pointed at.
    pub fn remove_path(&mut self, path: &str) -> Option<Entity> {
        let e = self.by_path.remove(path)?;
        self.by_entity.remove(&e);
        for prefix in prefixes(path) {
            let remove_prefix = self.by_prefix.get_mut(&prefix).is_some_and(|paths| {
                paths.remove(path);
                paths.is_empty()
            });
            if remove_prefix {
                self.by_prefix.remove(&prefix);
            }
        }
        Some(e)
    }

    /// Remove an entity's mapping (e.g. on despawn).
    pub fn remove_entity(&mut self, entity: Entity) -> Option<String> {
        let p = self.by_entity.get(&entity).cloned()?;
        self.remove_path(&p);
        Some(p)
    }

    /// Every `(path, entity)` currently mapped.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Entity)> {
        self.by_path.iter().map(|(p, e)| (p.as_str(), *e))
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Every `(path, entity)` whose path is `prefix` or a descendant of it —
    /// the set a `resynced` parent invalidates.
    pub fn subtree(&self, prefix: &str) -> Vec<(String, Entity)> {
        let norm = normalize_prim_path(prefix);
        let mut paths = self
            .by_prefix
            .get(&norm)
            .into_iter()
            .flat_map(|paths| paths.iter())
            .filter_map(|path| {
                self.by_path
                    .get(path)
                    .copied()
                    .map(|entity| (path.clone(), entity))
            })
            .filter(|(path, _)| is_descendant_or_self(&norm, path))
            .collect::<Vec<_>>();
        paths.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        paths
    }
}

fn prefixes(path: &str) -> Vec<String> {
    if path == "/" {
        return vec!["/".to_string()];
    }
    let mut output = vec!["/".to_string()];
    let mut current = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        current.push('/');
        current.push_str(segment);
        output.push(current.clone());
    }
    output
}
