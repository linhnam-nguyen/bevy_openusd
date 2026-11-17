use std::collections::HashMap;

use bevy::prelude::Entity;

/// Reverse index from a prim path to all scene-local occurrences of that path.
///
/// Native-instance projection can expose one semantic path under multiple
/// instance contexts. Keeping this index separate from the hierarchy maps
/// makes path-only presentation lookup proportional to its actual matches.
#[derive(Debug, Default)]
pub(crate) struct SceneOccurrenceIndex {
    by_prim_path: HashMap<String, Vec<Entity>>,
}

impl SceneOccurrenceIndex {
    pub(crate) fn insert(&mut self, prim_path: &str, entity: Entity) {
        self.by_prim_path
            .entry(prim_path.to_owned())
            .or_default()
            .push(entity);
    }

    pub(crate) fn resolve(&self, prim_path: &str) -> &[Entity] {
        self.by_prim_path.get(prim_path).map_or(&[], Vec::as_slice)
    }
}
