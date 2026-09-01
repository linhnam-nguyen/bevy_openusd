//! Reverse dependency index for native OpenUSD instance proxies.

use anyhow::Result;
use bevy::prelude::Resource;
use openusd::sdf::Path;
use openusd::usd::{EditTargetArc, PrimPredicate, Stage};
use std::collections::{HashMap, HashSet};

/// Maps prototype/source paths to scene-scoped instance proxy paths.
///
/// The index stores paths, not entities. A prototype can be shared by many
/// instance roots, and Bevy entities are replaceable during reconciliation.
#[derive(Resource, Debug, Default)]
pub struct NativeInstanceDependencyIndex {
    by_proxy: HashMap<String, HashSet<String>>,
    by_prototype: HashMap<String, HashSet<String>>,
    by_prototype_prefix: HashMap<String, HashSet<String>>,
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
    pub(crate) fn rebuild(&mut self, stage: &Stage) -> Result<()> {
        let mut proxies = Vec::new();
        stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
            proxies.push(path.as_str().to_string());
        })?;
        self.clear();
        for path in proxies {
            self.refresh_path(stage, &path);
        }
        Ok(())
    }

    /// Refresh one path after a scoped reconcile; no stage-wide scan occurs.
    pub(crate) fn refresh_path(&mut self, stage: &Stage, proxy: &str) {
        self.remove_proxy(proxy);
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
            .map(|prim| prim.path().as_str().to_string())
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
                    keys.insert(source.as_str().to_string());
                }
            }
        }
        for key in &keys {
            self.by_prototype
                .entry(key.clone())
                .or_default()
                .insert(proxy.to_string());
            for prefix in prefixes(key) {
                self.by_prototype_prefix
                    .entry(prefix)
                    .or_default()
                    .insert(proxy.to_string());
            }
        }
        self.by_proxy.insert(proxy.to_string(), keys);
    }

    /// Remove one proxy and all reverse edges pointing at it.
    pub(crate) fn remove_proxy(&mut self, proxy: &str) {
        let Some(keys) = self.by_proxy.remove(proxy) else {
            return;
        };
        for key in keys {
            let remove_key = self.by_prototype.get_mut(&key).is_some_and(|proxies| {
                proxies.remove(proxy);
                proxies.is_empty()
            });
            if remove_key {
                self.by_prototype.remove(&key);
            }
            for prefix in prefixes(&key) {
                let remove_prefix =
                    self.by_prototype_prefix
                        .get_mut(&prefix)
                        .is_some_and(|proxies| {
                            proxies.remove(proxy);
                            proxies.is_empty()
                        });
                if remove_prefix {
                    self.by_prototype_prefix.remove(&prefix);
                }
            }
        }
    }

    /// Remove every registered proxy, used when a progressive generation resets.
    pub(crate) fn clear(&mut self) {
        self.by_proxy.clear();
        self.by_prototype.clear();
        self.by_prototype_prefix.clear();
    }

    /// Return scene proxy paths affected by a changed prototype/source path.
    pub(crate) fn dependents_for_path(&self, changed: &str) -> HashSet<String> {
        let mut dependents = HashSet::new();
        for prefix in prefixes(changed) {
            if prefix == "/" && changed != "/" {
                continue;
            }
            if let Some(proxies) = self.by_prototype.get(&prefix) {
                dependents.extend(proxies.iter().cloned());
            }
        }
        if let Some(proxies) = self.by_prototype_prefix.get(changed) {
            dependents.extend(proxies.iter().cloned());
        }
        dependents
    }

    /// Return scene proxy paths whose prototype is covered by a resync root.
    pub(crate) fn dependents_for_resync_root(&self, root: &str) -> HashSet<String> {
        self.dependents_for_path(root)
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
