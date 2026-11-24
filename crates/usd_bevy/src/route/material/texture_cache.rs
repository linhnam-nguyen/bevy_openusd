use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::image::Image;
use bevy::platform::hash::FixedHasher;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, WgpuFeatures};
use bevy::render::renderer::RenderDevice;

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
    normal: bool,
    wrap: [u8; 2],
}

impl TextureCacheKey {
    /// Construct a cache key for an authored path and color-space variant.
    pub fn new(path: impl Into<String>, is_srgb: bool) -> Self {
        Self::with_normal(path, is_srgb, false, [0, 0])
    }
    fn with_normal(path: impl Into<String>, is_srgb: bool, normal: bool, wrap: [u8; 2]) -> Self {
        Self {
            path: path.into(),
            is_srgb,
            normal,
            wrap,
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
    pub(super) context_revision: u64,
    pub(super) decoded_images: HashMap<([u8; 2], u64), Handle<Image>>,
    // Equality checks original encoded bytes; hash equality alone is insufficient.
    pub(super) encoded: HashMap<(Arc<[u8]>, bool, bool, [u8; 2]), Handle<Image>>,
    pub(super) encoded_bytes: usize,
    pub(super) retained_image_bytes: usize,
    pub(super) authored_black_borders: HashMap<AssetId<Image>, [bool; 2]>,
}

impl UsdTextureCache {
    /// Transparent black axes authored in the USD sampler, including axes
    /// presented as edge clamp when black-border hardware support is absent.
    pub fn authored_black_border(&self, handle: &Handle<Image>) -> [bool; 2] {
        self.authored_black_borders
            .get(&handle.id())
            .copied()
            .unwrap_or_default()
    }

    /// Register authored sampler intent for a projected or cache-hydrated image.
    /// The caller must register the actual allocated handle, not a source path.
    pub fn remember_authored_black_border(&mut self, handle: &Handle<Image>, axes: [bool; 2]) {
        if axes.contains(&true) {
            self.authored_black_borders.insert(handle.id(), axes);
        } else {
            self.authored_black_borders.remove(&handle.id());
        }
    }

    /// Metadata owns no strong handles and survives only while its image is alive.
    pub fn prune_authored_black_borders(&mut self, images: &Assets<Image>) {
        self.authored_black_borders
            .retain(|id, _| images.contains(*id));
    }

    /// Snapshot hit/miss and loading counters for diagnostics or profiling.
    pub fn stats(&self) -> TextureCacheStats {
        self.stats
    }

    /// Clear profiling counters without dropping cached textures or archives.
    pub fn reset_stats(&mut self) {
        self.stats = TextureCacheStats::default();
    }

    /// Replace the packages that own the active stage's textures.
    ///
    /// Texture handles and archive indexes are stage-scoped derived state. A
    /// replacement invalidates both collections when ownership changes so a
    /// same inner path in a different USDZ cannot reuse the previous image.
    pub fn replace_active_archives(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut next = paths
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect::<Vec<_>>();
        next.sort();
        next.dedup();
        // This method is called at every authoritative stage activation. Even
        // an identical package list may contain changed bytes, so the image
        // handles and archive indexes must not survive the replacement.
        self.archive_paths = next;
        self.invalidate_textures();
        self.archive_indices.clear();
    }

    /// Drop every stage-owned package and its derived texture entries.
    pub fn clear_active_archives(&mut self) {
        self.replace_active_archives(std::iter::empty());
    }

    pub(crate) fn invalidate_textures(&mut self) {
        self.textures.clear();
        self.decoded_images.clear();
        // Encoded reuse and live projected/hydrated materials can outlive the
        // alias generation. Prune metadata by Image lifetime, not Stage lifetime.
        self.context_revision = self
            .context_revision
            .checked_add(1)
            .expect("texture context revision exhausted");
    }

    pub(super) fn archive_indices(&self) -> &HashMap<PathBuf, ArchiveIndex> {
        &self.archive_indices
    }

    pub(super) fn insert_archive_index(&mut self, path: PathBuf, index: ArchiveIndex) {
        self.archive_indices.insert(path, index);
    }
}

pub(super) fn resolve_sampled_texture(
    world: &mut World,
    path: &str,
    is_srgb: bool,
    normal: bool,
    wrap: [u8; 2],
) -> Option<Handle<Image>> {
    let border_supported = world.get_resource::<RenderDevice>().is_some_and(|device| {
        device
            .features()
            .contains(WgpuFeatures::ADDRESS_MODE_CLAMP_TO_BORDER)
    });
    let max_extent = world
        .get_resource::<RenderDevice>()
        .map(|device| device.limits().max_texture_dimension_2d);
    let black_axes = wrap.map(|mode| mode == 3);
    // A previously CPU-warmed image or an image from another device must not
    // bypass the current device's sampler/extent constraints.
    let compatible = |image: &Image| {
        let size = image.texture_descriptor.size;
        if max_extent.is_some_and(|max| size.width > max || size.height > max) {
            return false;
        }
        use bevy::image::{ImageAddressMode, ImageSampler};
        let ImageSampler::Descriptor(sampler) = &image.sampler else {
            return !black_axes.contains(&true);
        };
        let actual = [sampler.address_mode_u, sampler.address_mode_v]
            .map(|mode| mode == ImageAddressMode::ClampToBorder);
        actual == black_axes.map(|black| black && border_supported)
    };
    let key = TextureCacheKey::with_normal(path, is_srgb, normal, wrap);
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
                    .any(|candidate| candidate.path == path && candidate.is_srgb != is_srgb)
            });
    let cached_is_alive = cached.as_ref().is_some_and(|handle| {
        world
            .get_resource::<Assets<Image>>()
            .and_then(|images| images.get(handle))
            .is_some_and(&compatible)
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
    // Amortize the lifetime sweep across misses. Hydration also prunes once
    // per committed batch; no metadata owns a strong image handle.
    let prune_metadata = world
        .get_resource::<UsdTextureCache>()
        .is_some_and(|cache| cache.stats.misses % 64 == 1);
    if prune_metadata {
        world.resource_scope(|world, mut cache: Mut<UsdTextureCache>| {
            if let Some(images) = world.get_resource::<Assets<Image>>() {
                cache.prune_authored_black_borders(images);
            }
        });
    }
    let source = (Arc::<[u8]>::from(bytes), is_srgb, normal, wrap);
    let reused = world
        .get_resource::<UsdTextureCache>()
        .and_then(|cache| cache.encoded.get(&source))
        .cloned()
        .filter(|h| {
            world
                .resource::<Assets<Image>>()
                .get(h)
                .is_some_and(&compatible)
        });
    if let Some(handle) = reused {
        if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
            cache.remember_authored_black_border(&handle, black_axes);
            cache.textures.insert(key, handle.clone());
        }
        return Some(handle);
    }
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.stats.decode_calls += 1;
    }
    let Ok(img) = image::load_from_memory(&source.0) else {
        if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
            cache.stats.load_failures += 1;
        }
        return None;
    };
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    // Source projection is the fallback for rejected derived payloads. Do not
    // submit the same impossible extent to the GPU through the source route.
    if max_extent.is_some_and(|max| width > max || height > max) {
        bevy::log::warn!(
            "USD texture {path} ({width}x{height}) exceeds the active device extent; using the existing material fallback"
        );
        if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
            cache.stats.load_failures += 1;
        }
        return None;
    }

    let format = if is_srgb {
        TextureFormat::Rgba8UnormSrgb
    } else {
        TextureFormat::Rgba8Unorm
    };

    let mut bevy_image = Image::new(
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

    super::sampling::finalize(&mut bevy_image, is_srgb, normal);
    super::sampling::apply_wrap(&mut bevy_image, wrap, border_supported);
    let image_size = bevy_image.data.as_ref().map_or(0, Vec::len);
    let image_key = (
        wrap,
        FixedHasher.hash_one((width, height, format, normal, &bevy_image.data)),
    );
    let cached_image = world
        .get_resource::<UsdTextureCache>()
        .and_then(|cache| cache.decoded_images.get(&image_key))
        .filter(|handle| {
            world
                .resource::<Assets<Image>>()
                .get(*handle)
                .is_some_and(|image| {
                    image.texture_descriptor == bevy_image.texture_descriptor
                        && image.asset_usage == bevy_image.asset_usage
                        && image.sampler == bevy_image.sampler
                        && image.data == bevy_image.data
                })
        })
        .cloned();
    let handle =
        cached_image.unwrap_or_else(|| world.resource_mut::<Assets<Image>>().add(bevy_image));

    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        if cache.decoded_images.len() >= 256 && !cache.decoded_images.contains_key(&image_key) {
            cache.decoded_images.clear();
        }
        cache.decoded_images.insert(image_key, handle.clone());
        const SOURCE_BUDGET: usize = 128 * 1024 * 1024;
        const IMAGE_BUDGET: usize = 256 * 1024 * 1024;
        if source.0.len() <= SOURCE_BUDGET && image_size <= IMAGE_BUDGET {
            if cache.encoded.len() >= 256
                || cache.encoded_bytes.saturating_add(source.0.len()) > SOURCE_BUDGET
                || cache.retained_image_bytes.saturating_add(image_size) > IMAGE_BUDGET
            {
                cache.encoded.clear();
                cache.encoded_bytes = 0;
                cache.retained_image_bytes = 0;
            }
            if !cache.encoded.contains_key(&source) {
                cache.encoded_bytes += source.0.len();
                cache.retained_image_bytes += image_size;
            }
            cache.encoded.insert(source, handle.clone());
        }
        cache.remember_authored_black_border(&handle, black_axes);
        cache.textures.insert(key, handle.clone());
    }

    Some(handle)
}
