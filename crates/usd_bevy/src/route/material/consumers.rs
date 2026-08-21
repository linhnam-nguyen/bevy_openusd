use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

/// Reverse index from a composed Material path to the gprims consuming it.
///
/// The index is maintained while gprims are projected, so a shader edit can
/// target only affected consumers instead of scanning every projected prim.
#[derive(Resource, Default)]
pub(crate) struct MaterialConsumerIndex {
    by_material: HashMap<String, HashSet<String>>,
    by_consumer: HashMap<String, (String, Entity)>,
}

impl MaterialConsumerIndex {
    pub(super) fn update(&mut self, consumer: &str, binding: Option<&str>, entity: Entity) {
        let next = binding.map(str::to_owned);
        if self
            .by_consumer
            .get(consumer)
            .is_some_and(|(current, _)| Some(current) == next.as_ref())
        {
            if let Some((_, current_entity)) = self.by_consumer.get_mut(consumer) {
                *current_entity = entity;
            }
            return;
        }
        if let Some((previous, _)) = self.by_consumer.remove(consumer) {
            self.remove_from_material(&previous, consumer);
        }
        if let Some(binding) = next {
            self.by_material
                .entry(binding.clone())
                .or_default()
                .insert(consumer.to_owned());
            self.by_consumer
                .insert(consumer.to_owned(), (binding, entity));
        }
    }

    pub(super) fn consumer_entities_for(&self, changed_path: &str) -> Vec<(String, Entity)> {
        let mut consumer_paths = HashSet::new();
        let mut path = changed_path.to_owned();
        loop {
            if let Some(entries) = self.by_material.get(&path) {
                consumer_paths.extend(entries.iter().cloned());
            }
            if path == "/" {
                break;
            }
            let parent = path.rfind('/').unwrap_or(0);
            path.truncate(parent.max(1));
        }
        consumer_paths
            .into_iter()
            .filter_map(|path| {
                self.by_consumer
                    .get(&path)
                    .map(|(_, entity)| (path, *entity))
            })
            .collect()
    }

    fn remove_from_material(&mut self, binding: &str, consumer: &str) {
        let Some(consumers) = self.by_material.get_mut(binding) else {
            return;
        };
        consumers.remove(consumer);
        if consumers.is_empty() {
            self.by_material.remove(binding);
        }
    }
}
