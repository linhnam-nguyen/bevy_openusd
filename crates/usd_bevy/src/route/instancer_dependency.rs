//! Reverse dependency index for PointInstancer prototype subtrees.

use bevy::prelude::Resource;
use std::collections::{HashMap, HashSet};

use crate::live::{PathId, PathStore};

/// Maps prototype path IDs to the PointInstancer prims that consume them.
///
/// The index stores compact IDs; path bytes are owned by the shared
/// [`PathStore`]. Prototype edits arrive as USD paths, while Bevy entities are
/// only a replaceable projection detail.
#[derive(Resource, Debug, Default)]
pub struct PointInstancerDependencyIndex {
    by_instancer: HashMap<PathId, HashSet<PathId>>,
    by_prototype: HashMap<PathId, HashSet<PathId>>,
}

impl PointInstancerDependencyIndex {
    pub(crate) fn replace_instancer(
        &mut self,
        paths: &mut PathStore,
        instancer: impl AsRef<str>,
        prototype_roots: impl IntoIterator<Item = String>,
    ) {
        let instancer = paths.intern(instancer);
        self.remove_instancer_id(instancer);
        let roots = prototype_roots
            .into_iter()
            .map(|root| paths.intern(root))
            .collect::<HashSet<_>>();
        for root in &roots {
            self.by_prototype
                .entry(*root)
                .or_default()
                .insert(instancer);
        }
        self.by_instancer.insert(instancer, roots);
    }

    pub(crate) fn remove_instancer(&mut self, paths: &PathStore, instancer: &str) {
        let Some(instancer) = paths.lookup(instancer) else {
            return;
        };
        self.remove_instancer_id(instancer);
    }

    fn remove_instancer_id(&mut self, instancer: PathId) {
        let Some(roots) = self.by_instancer.remove(&instancer) else {
            return;
        };
        for root in roots {
            let mut remove_root = false;
            if let Some(consumers) = self.by_prototype.get_mut(&root) {
                consumers.remove(&instancer);
                remove_root = consumers.is_empty();
            }
            if remove_root {
                self.by_prototype.remove(&root);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.by_instancer.clear();
        self.by_prototype.clear();
    }

    /// Return consumers whose registered prototype root contains a changed prim.
    pub(crate) fn dependents_for_path(
        &self,
        paths: &PathStore,
        changed_path: &str,
    ) -> HashSet<PathId> {
        let mut consumers = HashSet::new();
        paths.for_each_ancestor_id(changed_path, |ancestor| {
            if let Some(instancers) = self.by_prototype.get(&ancestor) {
                consumers.extend(instancers.iter().copied());
            }
        });
        consumers
    }

    /// Return consumers whose registered prototype root is covered by a
    /// resync boundary, in either direction.
    ///
    /// Resync notifications describe a composition boundary rather than
    /// necessarily the exact prim that changed. Property changes use
    /// [`Self::dependents_for_path`] instead so an instancer's own transform
    /// edit cannot be mistaken for a prototype dependency.
    pub(crate) fn dependents_for_resync_root(
        &self,
        paths: &PathStore,
        resync_root: &str,
    ) -> HashSet<PathId> {
        let Some(resync_root) = paths.lookup(resync_root) else {
            return HashSet::new();
        };
        self.by_prototype
            .iter()
            .filter(|(root, _)| {
                paths.is_descendant_or_self(**root, resync_root)
                    || paths.is_descendant_or_self(resync_root, **root)
            })
            .flat_map(|(_, consumers)| consumers.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_resolve_through_compact_ids() {
        let mut paths = PathStore::default();
        let mut index = PointInstancerDependencyIndex::default();
        index.replace_instancer(
            &mut paths,
            "/World/Instancer",
            vec!["/World/Prototypes/Tree".to_owned()],
        );

        let dependents = index.dependents_for_path(&paths, "/World/Prototypes/Tree/Trunk");
        assert_eq!(
            dependents
                .iter()
                .filter_map(|id| paths.path(*id))
                .collect::<Vec<_>>(),
            vec!["/World/Instancer"]
        );
    }
}
