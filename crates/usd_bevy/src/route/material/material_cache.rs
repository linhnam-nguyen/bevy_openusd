use std::collections::HashMap;

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
    pub(super) stats: MaterialCacheStats,
}

impl UsdMaterialCache {
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
