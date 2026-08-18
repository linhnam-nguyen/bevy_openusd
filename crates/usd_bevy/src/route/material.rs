//! Material route: a gprim's bound `UsdShade` Material → [`StandardMaterial`].
//!
//! Runs after the mesh route, replacing the placeholder material the mesh route
//! attaches. Reads the `material:binding` and decodes the bound
//! `UsdPreviewSurface` (and Omni/MaterialX equivalents) via [`read::shade`].
//! Resolves textures from the filesystem or embedded `.usdz` archives into [`Assets<Image>`].

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::{PrimRoute, RouteCtx};
use crate::read::shade::{ReadPreviewMaterial, read_material_binding, read_preview_material};

/// Maps a bound Material → the entity's [`MeshMaterial3d`].
pub struct MaterialRoute;

/// Counters collected by [`UsdTextureCache`] so texture-cache changes can be
/// based on observed hit/miss and loading behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextureCacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub stale_handles: u64,
    pub load_failures: u64,
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

/// Cache of loaded USD textures keyed by authored asset path and color space.
#[derive(Resource, Default)]
pub struct UsdTextureCache {
    pub textures: HashMap<TextureCacheKey, Handle<Image>>,
    pub archive_paths: Vec<PathBuf>,
    archive_indices: HashMap<PathBuf, ArchiveIndex>,
    stats: TextureCacheStats,
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ArchiveLookupStats {
    archives_scanned: u64,
    entries_scanned: u64,
    hits: u64,
    misses: u64,
    index_builds: u64,
    index_invalidations: u64,
    entries_indexed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArchiveFingerprint {
    length: u64,
    modified_ns: Option<u128>,
}

impl ArchiveFingerprint {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        Some(Self {
            length: metadata.len(),
            modified_ns,
        })
    }
}

#[derive(Clone, Debug)]
struct ArchiveIndex {
    fingerprint: ArchiveFingerprint,
    entries: HashMap<String, String>,
}

#[derive(Clone)]
struct CachedMaterial {
    descriptor: ReadPreviewMaterial,
    handle: Handle<StandardMaterial>,
}

/// Cache of decoded materials keyed by their composed USD Material path.
///
/// The descriptor is retained with the handle so a live material edit creates
/// a fresh asset instead of returning a stale cached handle.
#[derive(Resource, Default)]
pub struct UsdMaterialCache {
    materials: HashMap<String, CachedMaterial>,
    stats: MaterialCacheStats,
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

/// The prim's decoded preview material, if it has a binding that resolves.
fn material_of(ctx: &RouteCtx) -> Option<(String, ReadPreviewMaterial)> {
    let binding = read_material_binding(ctx.stage, ctx.path).ok().flatten()?;
    let material = read_preview_material(ctx.stage, &binding).ok().flatten()?;
    Some((binding.as_str().to_owned(), material))
}

fn normalized_archive_entry(name: &str) -> String {
    name.trim_start_matches("./")
        .trim_start_matches('/')
        .to_owned()
}

fn archive_entry_matches(entry: &str, requested: &str) -> bool {
    entry == requested || entry.ends_with(requested) || requested.ends_with(entry)
}

fn canonical_archive_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn push_unique_usdz(files: &mut Vec<PathBuf>, path: PathBuf) {
    let path = canonical_archive_path(&path);
    if !files.contains(&path) {
        files.push(path);
    }
}

fn collect_usdz_files(world: &World, manifest_dir: &Path) -> Vec<PathBuf> {
    let mut usdz_files = Vec::new();
    if let Some(cache) = world.get_resource::<UsdTextureCache>() {
        for path in &cache.archive_paths {
            if path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("usdz"))
            {
                push_unique_usdz(&mut usdz_files, path.clone());
            }
        }
    }

    let search_dirs = [
        manifest_dir.join("assets/external"),
        manifest_dir.join("assets"),
        PathBuf::from("assets/external"),
        PathBuf::from("assets"),
    ];
    for dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("usdz")
                }) {
                    push_unique_usdz(&mut usdz_files, path);
                }
            }
        }
    }
    usdz_files
}

fn build_archive_index(path: &Path, fingerprint: ArchiveFingerprint) -> (ArchiveIndex, u64) {
    let mut entries = HashMap::new();
    let Ok(file) = std::fs::File::open(path) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return (
            ArchiveIndex {
                fingerprint,
                entries,
            },
            0,
        );
    };

    let entries_scanned = archive.len() as u64;
    for index in 0..archive.len() {
        let Ok(zip_file) = archive.by_index(index) else {
            continue;
        };
        let original_name = zip_file.name().to_owned();
        entries
            .entry(normalized_archive_entry(&original_name))
            .or_insert(original_name);
    }

    (
        ArchiveIndex {
            fingerprint,
            entries,
        },
        entries_scanned,
    )
}

fn ensure_archive_index(world: &mut World, path: &Path) -> ArchiveLookupStats {
    let Some(fingerprint) = ArchiveFingerprint::read(path) else {
        return ArchiveLookupStats::default();
    };
    let path = canonical_archive_path(path);
    let (needs_build, invalidated) = world
        .get_resource::<UsdTextureCache>()
        .map(|cache| match cache.archive_indices.get(&path) {
            Some(index) if index.fingerprint == fingerprint => (false, false),
            Some(_) => (true, true),
            None => (true, false),
        })
        .unwrap_or((false, false));
    if !needs_build {
        return ArchiveLookupStats::default();
    }

    let (index, entries_scanned) = build_archive_index(&path, fingerprint);
    let entries_indexed = index.entries.len() as u64;
    if let Some(mut cache) = world.get_resource_mut::<UsdTextureCache>() {
        cache.archive_indices.insert(path, index);
    }
    ArchiveLookupStats {
        archives_scanned: 1,
        entries_scanned,
        index_builds: 1,
        index_invalidations: u64::from(invalidated),
        entries_indexed,
        ..Default::default()
    }
}

fn scan_archives_without_index(
    usdz_files: &[PathBuf],
    norm_path: &str,
    archive_stats: &mut ArchiveLookupStats,
) -> Option<Vec<u8>> {
    for usdz in usdz_files {
        archive_stats.archives_scanned += 1;
        let Ok(file) = std::fs::File::open(usdz) else {
            continue;
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            continue;
        };

        for index in 0..archive.len() {
            archive_stats.entries_scanned += 1;
            let Ok(mut zip_file) = archive.by_index(index) else {
                continue;
            };
            let norm_zip = normalized_archive_entry(zip_file.name());
            if archive_entry_matches(&norm_zip, norm_path) {
                let mut buffer = Vec::new();
                if zip_file.read_to_end(&mut buffer).is_ok() {
                    archive_stats.hits += 1;
                    return Some(buffer);
                }
            }
        }
    }
    None
}

fn read_texture_bytes(
    world: &mut World,
    texture_path: &str,
) -> (Option<Vec<u8>>, ArchiveLookupStats) {
    let mut archive_stats = ArchiveLookupStats::default();
    let raw_path = Path::new(texture_path);
    if raw_path.is_absolute() && raw_path.exists() {
        if let Ok(bytes) = std::fs::read(raw_path) {
            return (Some(bytes), archive_stats);
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join(texture_path),
        manifest_dir.join("assets").join(texture_path),
        manifest_dir.join("assets/external").join(texture_path),
        PathBuf::from(texture_path),
        PathBuf::from("assets").join(texture_path),
        PathBuf::from("assets/external").join(texture_path),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            if let Ok(bytes) = std::fs::read(candidate) {
                return (Some(bytes), archive_stats);
            }
        }
    }

    let norm_path = normalized_archive_entry(texture_path);
    let usdz_files = collect_usdz_files(world, &manifest_dir);
    if world.get_resource::<UsdTextureCache>().is_some() {
        for usdz in &usdz_files {
            let stats = ensure_archive_index(world, usdz);
            archive_stats.archives_scanned += stats.archives_scanned;
            archive_stats.entries_scanned += stats.entries_scanned;
            archive_stats.index_builds += stats.index_builds;
            archive_stats.index_invalidations += stats.index_invalidations;
            archive_stats.entries_indexed += stats.entries_indexed;
            let path = canonical_archive_path(usdz);
            let entry_name = world
                .get_resource::<UsdTextureCache>()
                .and_then(|cache| cache.archive_indices.get(&path))
                .and_then(|index| {
                    index
                        .entries
                        .iter()
                        .find(|(entry, _)| archive_entry_matches(entry, &norm_path))
                        .map(|(_, original)| original.clone())
                });
            let Some(entry_name) = entry_name else {
                continue;
            };
            let Ok(file) = std::fs::File::open(usdz) else {
                continue;
            };
            let Ok(mut archive) = zip::ZipArchive::new(file) else {
                continue;
            };
            let Ok(mut zip_file) = archive.by_name(&entry_name) else {
                continue;
            };
            let mut buffer = Vec::new();
            if zip_file.read_to_end(&mut buffer).is_ok() {
                archive_stats.hits += 1;
                return (Some(buffer), archive_stats);
            }
        }
    } else if let Some(bytes) =
        scan_archives_without_index(&usdz_files, &norm_path, &mut archive_stats)
    {
        return (Some(bytes), archive_stats);
    }

    if !usdz_files.is_empty() {
        archive_stats.misses = 1;
    }
    (None, archive_stats)
}

fn resolve_texture(world: &mut World, path: &str, is_srgb: bool) -> Option<Handle<Image>> {
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

fn build_standard_material(read: &ReadPreviewMaterial, world: &mut World) -> StandardMaterial {
    let mut m = StandardMaterial::default();
    if let Some(c) = read.diffuse_color {
        let a = read.opacity.unwrap_or(1.0);
        m.base_color = Color::srgba(c[0], c[1], c[2], a);
    } else if let Some(a) = read.opacity {
        m.base_color.set_alpha(a);
    }
    if read.opacity.is_some_and(|a| a < 1.0) {
        m.alpha_mode = AlphaMode::Blend;
    }
    if let Some(r) = read.roughness {
        m.perceptual_roughness = r;
    }
    if let Some(mtl) = read.metallic {
        m.metallic = mtl;
    }
    if let Some(e) = read.emissive_color {
        m.emissive = LinearRgba::rgb(e[0], e[1], e[2]);
    }
    if let Some(ior) = read.ior {
        m.ior = ior;
    }
    if let Some(uv) = &read.uv_transform {
        m.uv_transform = bevy::math::Affine2::from_scale_angle_translation(
            Vec2::from(uv.scale),
            uv.rotation_deg.to_radians(),
            Vec2::from(uv.translation),
        );
    }

    // Resolve textures from cache/disk/usdz
    if let Some(p) = &read.diffuse_texture {
        m.base_color_texture = resolve_texture(world, p, true);
    }
    if let Some(p) = &read.normal_texture {
        m.normal_map_texture = resolve_texture(world, p, false);
    }
    if let Some(p) = &read.metallic_texture {
        m.metallic_roughness_texture = resolve_texture(world, p, false);
    }
    if let Some(p) = &read.emissive_texture {
        m.emissive_texture = resolve_texture(world, p, true);
    }
    if let Some(p) = &read.occlusion_texture {
        m.occlusion_texture = resolve_texture(world, p, false);
    }

    m
}

fn intern_material(
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

impl PrimRoute for MaterialRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        read_material_binding(ctx.stage, ctx.path)
            .ok()
            .flatten()
            .is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<StandardMaterial>>().is_none() {
            return;
        }
        let Some((binding, descriptor)) = material_of(ctx) else {
            return;
        };
        let Some(handle) = intern_material(world, &binding, &descriptor) else {
            return;
        };
        if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
            mat.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(MeshMaterial3d(handle));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn write_archive_fixture(path: &Path, texture_names: &[&str]) {
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut bytes));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file("scene.usda", options).unwrap();
            writer.write_all(b"#usda 1.0").unwrap();
            for texture_name in texture_names {
                writer.start_file(texture_name, options).unwrap();
                writer.write_all(b"not an image").unwrap();
            }
            writer.finish().unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn stats_distinguish_texture_hit_and_failed_miss() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache::default());

        let handle = world.resource_mut::<Assets<Image>>().add(Image::default());
        world
            .resource_mut::<UsdTextureCache>()
            .textures
            .insert(TextureCacheKey::new("cached.png", true), handle.clone());

        assert_eq!(
            resolve_texture(&mut world, "cached.png", true),
            Some(handle)
        );
        assert!(resolve_texture(&mut world, "definitely-missing-texture.png", true).is_none());
        assert_eq!(
            world.resource::<UsdTextureCache>().stats(),
            TextureCacheStats {
                lookups: 2,
                hits: 1,
                misses: 1,
                stale_handles: 0,
                load_failures: 1,
                color_space_misses: 0,
                ..Default::default()
            }
        );
    }

    #[test]
    fn stale_texture_handle_is_not_returned() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache::default());

        let handle = world.resource_mut::<Assets<Image>>().add(Image::default());
        world
            .resource_mut::<UsdTextureCache>()
            .textures
            .insert(TextureCacheKey::new("removed.png", true), handle.clone());
        world.resource_mut::<Assets<Image>>().remove(handle.id());

        assert!(resolve_texture(&mut world, "removed.png", true).is_none());
        let stats = world.resource::<UsdTextureCache>().stats();
        assert_eq!(stats.lookups, 1);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.stale_handles, 1);
        assert_eq!(stats.load_failures, 1);
    }

    #[test]
    fn repository_texture_decode_is_cached() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache::default());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/external/franka/panda/DetailedProps/Materials/Textures/normal.png");
        let path = path.to_string_lossy().into_owned();

        let first = resolve_texture(&mut world, &path, false).expect("repository texture loads");
        let second =
            resolve_texture(&mut world, &path, false).expect("cached repository texture loads");

        assert_eq!(first, second);
        assert_eq!(
            world.resource::<UsdTextureCache>().stats(),
            TextureCacheStats {
                lookups: 2,
                hits: 1,
                misses: 1,
                stale_handles: 0,
                load_failures: 0,
                color_space_misses: 0,
                ..Default::default()
            }
        );
    }

    #[test]
    fn repository_usdz_texture_scan_is_cached() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let archive = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/external/usdz_sample.usdz")
            .canonicalize()
            .expect("repository USDZ fixture exists");
        world.insert_resource(UsdTextureCache {
            archive_paths: vec![archive],
            ..Default::default()
        });

        let first = resolve_texture(&mut world, "textures/checker.png", true)
            .expect("embedded repository texture loads");
        let second = resolve_texture(&mut world, "textures/checker.png", true)
            .expect("embedded repository texture cache hit");

        assert_eq!(first, second);
        assert_eq!(
            world.resource::<UsdTextureCache>().stats(),
            TextureCacheStats {
                lookups: 2,
                hits: 1,
                misses: 1,
                stale_handles: 0,
                load_failures: 0,
                color_space_misses: 0,
                archive_scans: 1,
                archive_entries_scanned: 2,
                archive_hits: 1,
                archive_misses: 0,
                archive_index_builds: 1,
                archive_index_invalidations: 0,
                archive_entries_indexed: 2,
            }
        );
    }

    #[test]
    fn repository_usdz_archive_index_is_reused_across_variants() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let archive = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/external/usdz_sample.usdz")
            .canonicalize()
            .expect("repository USDZ fixture exists");
        world.insert_resource(UsdTextureCache {
            archive_paths: vec![archive],
            ..Default::default()
        });

        let data_handle = resolve_texture(&mut world, "textures/checker.png", false)
            .expect("embedded data texture variant loads");
        let color_handle = resolve_texture(&mut world, "textures/checker.png", true)
            .expect("embedded sRGB texture variant loads");

        assert_ne!(data_handle, color_handle);
        let stats = world.resource::<UsdTextureCache>().stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.color_space_misses, 1);
        assert_eq!(stats.archive_scans, 1);
        assert_eq!(stats.archive_entries_scanned, 2);
        assert_eq!(stats.archive_hits, 2);
        assert_eq!(stats.archive_misses, 0);
        assert_eq!(stats.archive_index_builds, 1);
        assert_eq!(stats.archive_index_invalidations, 0);
        assert_eq!(stats.archive_entries_indexed, 2);
    }

    #[test]
    fn archive_index_invalidates_when_archive_changes() {
        let archive = std::env::temp_dir().join(format!(
            "usd_bevy_archive_index_{}.usdz",
            std::process::id()
        ));
        write_archive_fixture(&archive, &["textures/one.png"]);

        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache {
            archive_paths: vec![archive.clone()],
            ..Default::default()
        });

        assert!(resolve_texture(&mut world, "textures/one.png", true).is_none());
        write_archive_fixture(&archive, &["textures/one.png", "textures/two.png"]);
        assert!(resolve_texture(&mut world, "textures/two.png", true).is_none());

        let stats = world.resource::<UsdTextureCache>().stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.load_failures, 2);
        assert_eq!(stats.archive_scans, 2);
        assert_eq!(stats.archive_entries_scanned, 5);
        assert_eq!(stats.archive_hits, 2);
        assert_eq!(stats.archive_misses, 0);
        assert_eq!(stats.archive_index_builds, 2);
        assert_eq!(stats.archive_index_invalidations, 1);
        assert_eq!(stats.archive_entries_indexed, 5);
    }

    #[test]
    fn texture_cache_separates_color_space_variants() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache::default());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/external/franka/panda/DetailedProps/Materials/Textures/normal.png")
            .to_string_lossy()
            .into_owned();

        let data_handle =
            resolve_texture(&mut world, &path, false).expect("data texture variant loads");
        let color_handle =
            resolve_texture(&mut world, &path, true).expect("sRGB texture variant loads");

        assert_ne!(data_handle, color_handle);
        let images = world.resource::<Assets<Image>>();
        assert_eq!(
            images
                .get(&data_handle)
                .expect("data image remains cached")
                .texture_descriptor
                .format,
            TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            images
                .get(&color_handle)
                .expect("sRGB image remains cached")
                .texture_descriptor
                .format,
            TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            world.resource::<UsdTextureCache>().stats(),
            TextureCacheStats {
                lookups: 2,
                hits: 0,
                misses: 2,
                stale_handles: 0,
                load_failures: 0,
                color_space_misses: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn material_binding_cache_reuses_and_invalidates_descriptors() {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.insert_resource(UsdTextureCache::default());
        world.insert_resource(UsdMaterialCache::default());

        let first_descriptor = ReadPreviewMaterial {
            diffuse_color: Some([0.8, 0.1, 0.1]),
            ..Default::default()
        };
        let changed_descriptor = ReadPreviewMaterial {
            diffuse_color: Some([0.1, 0.8, 0.1]),
            ..Default::default()
        };
        let first = intern_material(&mut world, "/World/Materials/Shared", &first_descriptor)
            .expect("first material should be added");
        let reused = intern_material(&mut world, "/World/Materials/Shared", &first_descriptor)
            .expect("same material should be reused");
        let changed = intern_material(&mut world, "/World/Materials/Shared", &changed_descriptor)
            .expect("changed material should be rebuilt");

        assert_eq!(first, reused);
        assert_ne!(first, changed);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 2);
        assert_eq!(
            world.resource::<UsdMaterialCache>().stats(),
            MaterialCacheStats {
                lookups: 3,
                hits: 1,
                misses: 2,
                stale_handles: 0,
                descriptor_changes: 1,
            }
        );
    }
}
