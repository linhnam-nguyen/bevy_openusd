use std::collections::HashMap;
use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::archive::{ArchiveIndex, read_texture_bytes};

/// Counters collected by [`UsdTextureCache`] so texture-cache changes can be
/// based on observed hit/miss and loading behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextureCacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub stale_handles: u64,
    pub load_failures: u64,
    pub decode_calls: u64,
    pub color_space_misses: u64,
    pub archive_scans: u64,
    pub archive_entries_scanned: u64,
    pub archive_hits: u64,
    pub archive_misses: u64,
    pub archive_index_builds: u64,
    pub archive_index_invalidations: u64,
    pub archive_entries_indexed: u64,
}

/// A texture-cache key containing both the authored path and its color-space
/// interpretation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TextureCacheKey {
    /// The authored USD asset path.
    pub path: String,
    /// Whether the decoded image is intended for an sRGB color channel.
    pub is_srgb: bool,
}

impl TextureCacheKey {
    /// Construct a cache key for an authored path and color-space variant.
    pub fn new(path: impl Into<String>, is_srgb: bool) -> Self {
        Self {
            path: path.into(),
            is_srgb,
        }
    }
}

/// Cache of loaded USD textures keyed by authored asset path and color space.
#[derive(Resource, Default)]
pub struct UsdTextureCache {
    pub textures: HashMap<TextureCacheKey, Handle<Image>>,
    pub archive_paths: Vec<PathBuf>,
    pub(super) archive_indices: HashMap<PathBuf, ArchiveIndex>,
    pub(super) stats: TextureCacheStats,
}

impl UsdTextureCache {
    /// Snapshot hit/miss and loading counters for diagnostics or profiling.
    pub fn stats(&self) -> TextureCacheStats {
        self.stats
    }

    /// Clear profiling counters without dropping cached textures or archives.
    pub fn reset_stats(&mut self) {
        self.stats = TextureCacheStats::default();
    }

    pub(super) fn archive_indices(&self) -> &HashMap<PathBuf, ArchiveIndex> {
        &self.archive_indices
    }

    pub(super) fn insert_archive_index(&mut self, path: PathBuf, index: ArchiveIndex) {
        self.archive_indices.insert(path, index);
    }
}

pub(super) fn resolve_texture(
    world: &mut World,
    path: &str,
    is_srgb: bool,
) -> Option<Handle<Image>> {
    let key = TextureCacheKey::new(path, is_srgb);
    let cached = world
        .get_resource::<UsdTextureCache>()
        .and_then(|cache| cache.textures.get(&key).cloned());
    let color_space_mismatch = cached.is_none()
        && world
            .get_resource::<UsdTextureCache>()
            .is_some_and(|cache| {
                cache
                    .textures
                    .keys()
                    .any(|candidate| candidate.path == path)
            });
    let cached_is_alive = cached.as_ref().is_some_and(|handle| {
        world
            .get_resource::<Assets<Image>>()
            .is_some_and(|images| images.contains(handle))
    });
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.stats.lookups += 1;
        if cached_is_alive {
            cache.stats.hits += 1;
            return cached;
        }
        if cached.is_some() {
            cache.stats.stale_handles += 1;
        } else if color_space_mismatch {
            cache.stats.color_space_misses += 1;
        }
        cache.stats.misses += 1;
    }

    let (bytes, archive_stats) = read_texture_bytes(world, path);
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.stats.archive_scans += archive_stats.archives_scanned;
        cache.stats.archive_entries_scanned += archive_stats.entries_scanned;
        cache.stats.archive_hits += archive_stats.hits;
        cache.stats.archive_misses += archive_stats.misses;
        cache.stats.archive_index_builds += archive_stats.index_builds;
        cache.stats.archive_index_invalidations += archive_stats.index_invalidations;
        cache.stats.archive_entries_indexed += archive_stats.entries_indexed;
    }

    let Some(bytes) = bytes else {
        if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
            cache.stats.load_failures += 1;
        }
        return None;
    };
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.stats.decode_calls += 1;
    }
    let Ok(img) = image::load_from_memory(&bytes) else {
        if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
            cache.stats.load_failures += 1;
        }
        return None;
    };
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    let format = if is_srgb {
        TextureFormat::Rgba8UnormSrgb
    } else {
        TextureFormat::Rgba8Unorm
    };

    let bevy_image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.into_raw(),
        format,
        RenderAssetUsages::default(),
    );

    let handle = world.resource_mut::<Assets<Image>>().add(bevy_image);

    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.textures.insert(key, handle.clone());
    }

    Some(handle)
}
