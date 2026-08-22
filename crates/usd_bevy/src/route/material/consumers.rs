use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConsumerRecord {
    material: String,
    dependencies: Vec<String>,
    entity: Entity,
}

/// Reverse index from actual shading-network prim paths to gprims consuming
/// the corresponding Material.
///
/// The index is maintained while gprims are projected, so a shader or texture
/// edit can target only affected consumers instead of scanning every prim.
#[derive(Resource, Default)]
pub(crate) struct MaterialConsumerIndex {
    by_dependency: HashMap<String, HashSet<String>>,
    by_consumer: HashMap<String, ConsumerRecord>,
}

impl MaterialConsumerIndex {
    pub(super) fn update(
        &mut self,
        consumer: &str,
        binding: Option<&str>,
        dependencies: &[String],
        entity: Entity,
    ) {
        let next = binding.map(|material| {
            let mut dependencies = dependencies.to_vec();
            if !dependencies.iter().any(|path| path == material) {
                dependencies.push(material.to_owned());
                dependencies.sort_unstable();
            }
            (material.to_owned(), dependencies)
        });
        if self.by_consumer.get(consumer).is_some_and(|current| {
            next.as_ref().is_some_and(|(material, dependencies)| {
                current.material == *material && current.dependencies == *dependencies
            })
        }) {
            if let Some(current) = self.by_consumer.get_mut(consumer) {
                current.entity = entity;
            }
            return;
        }
        self.remove_consumer(consumer);
        if let Some((material, dependencies)) = next {
            for dependency in &dependencies {
                self.by_dependency
                    .entry(dependency.clone())
                    .or_default()
                    .insert(consumer.to_owned());
            }
            self.by_consumer.insert(
                consumer.to_owned(),
                ConsumerRecord {
                    material,
                    dependencies,
                    entity,
                },
            );
        }
    }

    pub(crate) fn remove_consumer(&mut self, consumer: &str) {
        let Some(record) = self.by_consumer.remove(consumer) else {
            return;
        };
        for dependency in &record.dependencies {
            self.remove_from_dependency(dependency, consumer);
        }
    }

    pub(super) fn consumer_entities_for(&self, changed_path: &str) -> Vec<(String, Entity)> {
        self.by_dependency
            .get(changed_path)
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter_map(|path| {
                self.by_consumer
                    .get(path)
                    .map(|record| (path.clone(), record.entity))
            })
            .collect()
    }

    fn remove_from_dependency(&mut self, dependency: &str, consumer: &str) {
        let Some(consumers) = self.by_dependency.get_mut(dependency) else {
            return;
        };
        consumers.remove(consumer);
        if consumers.is_empty() {
            self.by_dependency.remove(dependency);
        }
    }
}
