//! Reverse dependency index for PointInstancer prototype subtrees.

use bevy::prelude::Resource;
use std::collections::{HashMap, HashSet};

use crate::live::is_descendant_or_self;

/// Maps prototype prim paths to the PointInstancer prims that consume them.
///
/// The index is deliberately path based: prototype edits arrive as USD paths,
/// while Bevy entities are only a replaceable projection detail.
#[derive(Resource, Debug, Default)]
pub struct PointInstancerDependencyIndex {
    by_instancer: HashMap<String, HashSet<String>>,
    by_prototype: HashMap<String, HashSet<String>>,
    by_prototype_prefix: HashMap<String, HashSet<String>>,
}

impl PointInstancerDependencyIndex {
    pub(crate) fn replace_instancer(
        &mut self,
        instancer: impl Into<String>,
        prototype_roots: impl IntoIterator<Item = String>,
    ) {
        let instancer = instancer.into();
        self.remove_instancer(&instancer);
        let roots = prototype_roots.into_iter().collect::<HashSet<_>>();
        for root in &roots {
            self.by_prototype
                .entry(root.clone())
                .or_default()
                .insert(instancer.clone());
            for prefix in prefixes(root) {
                self.by_prototype_prefix
                    .entry(prefix)
                    .or_default()
                    .insert(instancer.clone());
            }
        }
        self.by_instancer.insert(instancer, roots);
    }

    pub(crate) fn remove_instancer(&mut self, instancer: &str) {
        let Some(roots) = self.by_instancer.remove(instancer) else {
            return;
        };
        for root in roots {
            let mut remove_root = false;
            if let Some(consumers) = self.by_prototype.get_mut(&root) {
                consumers.remove(instancer);
                remove_root = consumers.is_empty();
            }
            if remove_root {
                self.by_prototype.remove(&root);
            }
            for prefix in prefixes(&root) {
                let remove_prefix =
                    self.by_prototype_prefix
                        .get_mut(&prefix)
                        .is_some_and(|consumers| {
                            consumers.remove(instancer);
                            consumers.is_empty()
                        });
                if remove_prefix {
                    self.by_prototype_prefix.remove(&prefix);
                }
            }
        }
    }

    /// Return consumers whose registered prototype root contains a changed prim.
    pub(crate) fn dependents_for_path(&self, changed_path: &str) -> HashSet<String> {
        prefixes(changed_path)
            .into_iter()
            .filter(|prefix| prefix != "/" || changed_path == "/")
            .fold(HashSet::new(), |mut output, prefix| {
                if let Some(consumers) = self.by_prototype.get(&prefix) {
                    output.extend(consumers.iter().cloned());
                }
                output
            })
    }

    /// Return consumers whose registered prototype root is covered by a
    /// resync boundary, in either direction.
    ///
    /// Resync notifications describe a composition boundary rather than
    /// necessarily the exact prim that changed. Property changes use
    /// [`Self::dependents_for_path`] instead so an instancer's own transform
    /// edit cannot be mistaken for a prototype dependency.
    pub(crate) fn dependents_for_resync_root(&self, resync_root: &str) -> HashSet<String> {
        self.by_prototype
            .iter()
            .filter(|(root, _)| {
                is_descendant_or_self(root, resync_root) || is_descendant_or_self(resync_root, root)
            })
            .flat_map(|(_, consumers)| consumers.iter().cloned())
            .collect()
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
