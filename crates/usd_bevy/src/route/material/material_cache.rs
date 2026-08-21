use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::read::shade::ReadPreviewMaterial;

use super::builder::build_standard_material;

/// Counters collected by [`UsdMaterialCache`] for binding reuse and live
/// descriptor changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaterialCacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub stale_handles: u64,
    pub descriptor_changes: u64,
    pub retired_assets: u64,
    pub cleaned_assets: u64,
    pub cleanup_passes: u64,
    pub cleanup_entities_scanned: u64,
}

#[derive(Clone)]
pub(super) struct CachedMaterial {
    pub(super) descriptor: ReadPreviewMaterial,
    pub(super) handle: Handle<StandardMaterial>,
}

/// Cache of decoded materials keyed by their composed USD Material path.
///
/// The descriptor is retained with the handle so a live material edit creates
/// a fresh asset instead of returning a stale cached handle.
#[derive(Resource, Default)]
pub struct UsdMaterialCache {
    pub(super) materials: HashMap<String, CachedMaterial>,
    pub(super) retired_handles: Vec<Handle<StandardMaterial>>,
    pub(super) stats: MaterialCacheStats,
}

impl UsdMaterialCache {
    /// Number of composed Material paths currently interned.
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Whether no composed material paths are interned.
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    /// Snapshot material-cache counters for diagnostics or profiling.
    pub fn stats(&self) -> MaterialCacheStats {
        self.stats
    }

    /// Clear profiling counters without dropping cached material handles.
    pub fn reset_stats(&mut self) {
        self.stats = MaterialCacheStats::default();
    }
}

pub(super) fn intern_material(
    world: &mut World,
    binding: &str,
    descriptor: &ReadPreviewMaterial,
) -> Option<Handle<StandardMaterial>> {
    world.get_resource::<Assets<StandardMaterial>>()?;

    let cached = world
        .get_resource::<UsdMaterialCache>()
        .and_then(|cache| cache.materials.get(binding).cloned());
    let asset_is_alive = cached.as_ref().is_some_and(|entry| {
        world
            .resource::<Assets<StandardMaterial>>()
            .contains(&entry.handle)
    });
    let descriptor_matches = cached
        .as_ref()
        .is_some_and(|entry| entry.descriptor == *descriptor);

    if let Some(mut cache) = world.get_resource_mut::<UsdMaterialCache>() {
        cache.stats.lookups += 1;
        if asset_is_alive && descriptor_matches {
            cache.stats.hits += 1;
            return cached.map(|entry| entry.handle);
        }
        cache.stats.misses += 1;
        if cached.is_some() {
            if !asset_is_alive {
                cache.stats.stale_handles += 1;
            } else if !descriptor_matches {
                cache.stats.descriptor_changes += 1;
                if let Some(entry) = cached.as_ref() {
                    cache.retired_handles.push(entry.handle.clone());
                    cache.stats.retired_assets += 1;
                }
            }
        }
    }

    let material = build_standard_material(descriptor, world);
    let handle = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(material);
    if let Some(mut cache) = world.get_resource_mut::<UsdMaterialCache>() {
        cache.materials.insert(
            binding.to_owned(),
            CachedMaterial {
                descriptor: descriptor.clone(),
                handle: handle.clone(),
            },
        );
    }
    Some(handle)
}

/// Remove retired material assets only after no projected mesh references them.
pub(super) fn cleanup_retired_materials(world: &mut World) {
    let retired = world
        .get_resource_mut::<UsdMaterialCache>()
        .map(|mut cache| std::mem::take(&mut cache.retired_handles))
        .unwrap_or_default();
    if retired.is_empty() {
        return;
    }

    let (referenced, scanned) = {
        let mut query = world.query::<&MeshMaterial3d<StandardMaterial>>();
        let mut referenced = HashSet::new();
        let mut scanned = 0;
        for material in query.iter(world) {
            scanned += 1;
            referenced.insert(material.0.id());
        }
        (referenced, scanned)
    };
    let mut retained = Vec::new();
    let mut cleaned = 0;
    {
        let Some(mut assets) = world.get_resource_mut::<Assets<StandardMaterial>>() else {
            return;
        };
        for handle in retired {
            if referenced.contains(&handle.id()) {
                retained.push(handle);
            } else if assets.remove(handle.id()).is_some() {
                cleaned += 1;
            }
        }
    }
    if let Some(mut cache) = world.get_resource_mut::<UsdMaterialCache>() {
        cache.retired_handles.extend(retained);
        cache.stats.cleaned_assets += cleaned;
        cache.stats.cleanup_passes += 1;
        cache.stats.cleanup_entities_scanned += scanned;
    }
}
