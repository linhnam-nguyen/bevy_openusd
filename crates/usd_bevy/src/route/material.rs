//! Material route: a gprim's bound `UsdShade` Material → [`StandardMaterial`].
//!
//! Runs after the mesh route, replacing the placeholder material the mesh route
//! attaches. Reads the `material:binding` and decodes the bound
//! `UsdPreviewSurface` (and Omni/MaterialX equivalents) via [`read::shade`].
//! Resolves textures from the filesystem or embedded `.usdz` archives into [`Assets<Image>`].

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

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

/// Cache of loaded USD textures keyed by authored asset path.
#[derive(Resource, Default)]
pub struct UsdTextureCache {
    pub textures: HashMap<String, Handle<Image>>,
    pub archive_paths: Vec<PathBuf>,
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

fn read_texture_bytes(world: &World, texture_path: &str) -> Option<Vec<u8>> {
    let raw_path = Path::new(texture_path);
    if raw_path.is_absolute() && raw_path.exists() {
        if let Ok(bytes) = std::fs::read(raw_path) {
            return Some(bytes);
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
                return Some(bytes);
            }
        }
    }

    // Search inside USDZ archives
    let norm_path = texture_path
        .trim_start_matches("./")
        .trim_start_matches('/');

    let mut usdz_files = Vec::new();
    if let Some(cache) = world.get_resource::<UsdTextureCache>() {
        usdz_files.extend(cache.archive_paths.clone());
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
                let p = entry.path();
                if p.extension().is_some_and(|ext| ext == "usdz") && !usdz_files.contains(&p) {
                    usdz_files.push(p);
                }
            }
        }
    }

    for usdz in usdz_files {
        let Ok(file) = std::fs::File::open(&usdz) else {
            continue;
        };
        let Ok(mut archive) = zip::ZipArchive::new(file) else {
            continue;
        };

        for i in 0..archive.len() {
            let Ok(mut zip_file) = archive.by_index(i) else {
                continue;
            };
            let name = zip_file.name().to_string();
            let norm_zip = name.trim_start_matches("./").trim_start_matches('/');
            if norm_zip == norm_path
                || norm_zip.ends_with(norm_path)
                || norm_path.ends_with(norm_zip)
            {
                let mut buffer = Vec::new();
                if zip_file.read_to_end(&mut buffer).is_ok() {
                    return Some(buffer);
                }
            }
        }
    }

    None
}

fn resolve_texture(world: &mut World, path: &str, is_srgb: bool) -> Option<Handle<Image>> {
    let cached = world
        .get_resource::<UsdTextureCache>()
        .and_then(|cache| cache.textures.get(path).cloned());
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
        }
        cache.stats.misses += 1;
    }

    let Some(bytes) = read_texture_bytes(world, path) else {
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
        cache.textures.insert(path.to_string(), handle.clone());
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
    use super::*;

    #[test]
    fn stats_distinguish_texture_hit_and_failed_miss() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(UsdTextureCache::default());

        let handle = world.resource_mut::<Assets<Image>>().add(Image::default());
        world
            .resource_mut::<UsdTextureCache>()
            .textures
            .insert("cached.png".to_owned(), handle.clone());

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
            .insert("removed.png".to_owned(), handle.clone());
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
