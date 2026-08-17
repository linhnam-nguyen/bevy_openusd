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

/// Cache of loaded USD textures keyed by authored asset path.
#[derive(Resource, Default)]
pub struct UsdTextureCache {
    pub textures: HashMap<String, Handle<Image>>,
    pub archive_paths: Vec<PathBuf>,
}

/// The prim's decoded preview material, if it has a binding that resolves.
fn material_of(ctx: &RouteCtx) -> Option<ReadPreviewMaterial> {
    let binding = read_material_binding(ctx.stage, ctx.path).ok().flatten()?;
    read_preview_material(ctx.stage, &binding).ok().flatten()
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
    if let Some(cache) = world.get_resource::<UsdTextureCache>() {
        if let Some(handle) = cache.textures.get(path) {
            return Some(handle.clone());
        }
    }

    let bytes = read_texture_bytes(world, path)?;
    let img = image::load_from_memory(&bytes).ok()?;
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
        let Some(read) = material_of(ctx) else {
            return;
        };
        let material = build_standard_material(&read, world);
        let handle = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material);
        if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
            mat.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(MeshMaterial3d(handle));
        }
    }
}
