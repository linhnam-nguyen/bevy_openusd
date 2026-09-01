//! Reverse dependency index for native OpenUSD instance proxies.

use anyhow::Result;
use bevy::prelude::Resource;
use openusd::sdf::Path;
use openusd::usd::{EditTargetArc, PrimPredicate, Stage};
use std::collections::{HashMap, HashSet};

use super::path::{PathId, PathStore};

/// Maps prototype/source path IDs to scene-scoped instance proxy path IDs.
///
/// The index stores compact IDs, not path strings or entities. A prototype can
/// be shared by many instance roots, and Bevy entities are replaceable during
/// reconciliation. Path bytes are owned by the shared [`PathStore`].
#[derive(Resource, Debug, Default)]
pub struct NativeInstanceDependencyIndex {
    by_proxy: HashMap<PathId, HashSet<PathId>>,
    by_prototype: HashMap<PathId, HashSet<PathId>>,
}

impl NativeInstanceDependencyIndex {
    /// Number of registered scene proxy paths.
    pub fn len(&self) -> usize {
        self.by_proxy.len()
    }

    /// Whether no scene proxy paths are registered.
    pub fn is_empty(&self) -> bool {
        self.by_proxy.is_empty()
    }

    /// Rebuild the index after initial projection or an explicit full reconcile.
    pub(crate) fn rebuild(&mut self, paths: &mut PathStore, stage: &Stage) -> Result<()> {
        let mut proxies = Vec::new();
        stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
            proxies.push(path.as_str().to_string());
        })?;
        self.clear();
        for path in proxies {
            self.refresh_path(paths, stage, &path);
        }
        Ok(())
    }

    /// Refresh one path after a scoped reconcile; no stage-wide scan occurs.
    pub(crate) fn refresh_path(&mut self, paths: &mut PathStore, stage: &Stage, proxy: &str) {
        let proxy_id = paths.intern(proxy);
        self.remove_proxy_id(proxy_id);
        let Ok(path) = openusd::sdf::path(proxy) else {
            return;
        };
        let prim = stage.prim(path.clone());
        if !prim.is_instance_proxy().unwrap_or(false) {
            return;
        }
        let Some(prototype) = prim
            .prim_in_prototype()
            .ok()
            .flatten()
            .map(|prim| paths.intern(prim.path().as_str()))
        else {
            return;
        };
        let mut keys = HashSet::from([prototype]);
        if let Some(instance) = instance_root(stage, &path) {
            for arc in [
                EditTargetArc::Reference,
                EditTargetArc::Payload,
                EditTargetArc::Inherit,
                EditTargetArc::Specialize,
            ] {
                let Ok(target) = instance.edit_target_for_arc(arc) else {
                    continue;
                };
                if let Some(source) = target.map_to_spec_path(&path) {
                    keys.insert(paths.intern(source.as_str()));
                }
            }
        }
        for key in &keys {
            self.by_prototype
                .entry(key.clone())
                .or_default()
                .insert(proxy_id);
        }
        self.by_proxy.insert(proxy_id, keys);
    }

    /// Remove one proxy and all reverse edges pointing at it.
    pub(crate) fn remove_proxy(&mut self, paths: &PathStore, proxy: &str) {
        let Some(proxy) = paths.lookup(proxy) else {
            return;
        };
        self.remove_proxy_id(proxy);
    }

    fn remove_proxy_id(&mut self, proxy: PathId) {
        let Some(keys) = self.by_proxy.remove(&proxy) else {
            return;
        };
        for key in keys {
            let remove_key = self.by_prototype.get_mut(&key).is_some_and(|proxies| {
                proxies.remove(&proxy);
                proxies.is_empty()
            });
            if remove_key {
                self.by_prototype.remove(&key);
            }
        }
    }

    /// Remove every registered proxy, used when a progressive generation resets.
    pub(crate) fn clear(&mut self) {
        self.by_proxy.clear();
        self.by_prototype.clear();
    }

    /// Return scene proxy path IDs affected by a changed prototype/source path.
    pub(crate) fn dependents_for_path(&self, paths: &PathStore, changed: &str) -> HashSet<PathId> {
        let mut dependents = HashSet::new();
        paths.for_each_ancestor_id(changed, |ancestor| {
            if let Some(proxies) = self.by_prototype.get(&ancestor) {
                dependents.extend(proxies.iter().copied());
            }
        });
        dependents
    }

    /// Return scene proxy path IDs whose prototype is covered by a resync root.
    pub(crate) fn dependents_for_resync_root(
        &self,
        paths: &PathStore,
        root: &str,
    ) -> HashSet<PathId> {
        let Some(root) = paths.lookup(root) else {
            return HashSet::new();
        };
        self.by_prototype
            .iter()
            .filter(|(prototype, _)| {
                paths.is_descendant_or_self(root, **prototype)
                    || paths.is_descendant_or_self(**prototype, root)
            })
            .flat_map(|(_, proxies)| proxies.iter().copied())
            .collect()
    }
}

fn instance_root(stage: &Stage, path: &Path) -> Option<openusd::usd::Prim> {
    let mut current = path.clone();
    while let Some(parent) = current.parent() {
        current = parent;
        if stage.prim(current.clone()).is_instance().unwrap_or(false) {
            return Some(stage.prim(current));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_avoids_descendant_prototype_scan() {
        let mut paths = PathStore::default();
        let prototype = paths.intern("/World/Prototype/Nested");
        let proxy = paths.intern("/World/Instance/Nested");
        let mut index = NativeInstanceDependencyIndex::default();
        index.by_prototype.insert(prototype, HashSet::from([proxy]));
        index.by_proxy.insert(proxy, HashSet::from([prototype]));

        assert!(
            index
                .dependents_for_path(&paths, "/World/Prototype")
                .is_empty(),
            "an exact change must not scan prototypes below the changed path"
        );
        assert_eq!(
            index.dependents_for_path(&paths, "/World/Prototype/Nested/Geometry"),
            HashSet::from([proxy])
        );
        assert_eq!(
            index.dependents_for_resync_root(&paths, "/World/Prototype"),
            HashSet::from([proxy])
        );
    }
}
