use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use crate::read::shade::ReadPreviewMaterial;

use super::material_cache::intern_material;
use super::texture_cache::resolve_texture;
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
            decode_calls: 0,
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
    assert_eq!(stats.decode_calls, 0);
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
            decode_calls: 1,
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

    let first = resolve_texture(&mut world, "./textures/checker.png", true)
        .expect("embedded repository texture loads");
    let second = resolve_texture(&mut world, "./textures/checker.png", true)
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
            decode_calls: 1,
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

    let data_handle = resolve_texture(&mut world, "./textures/checker.png", false)
        .expect("embedded data texture variant loads");
    let color_handle = resolve_texture(&mut world, "./textures/checker.png", true)
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
    assert_eq!(stats.decode_calls, 2);
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
    assert_eq!(stats.decode_calls, 2);
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
            decode_calls: 2,
            color_space_misses: 1,
            ..Default::default()
        }
    );
}

#[test]
fn changed_texture_path_gets_a_distinct_decode() {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(UsdTextureCache::default());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/external/franka/panda/DetailedProps/Materials/Textures");
    let first_path = root.join("normal.png").to_string_lossy().into_owned();
    let changed_path = root
        .join("Logo_Textures_Albedo.png")
        .to_string_lossy()
        .into_owned();

    let first = resolve_texture(&mut world, &first_path, false).expect("first texture loads");
    let changed = resolve_texture(&mut world, &changed_path, false).expect("changed texture loads");

    assert_ne!(
        first, changed,
        "authored path changes must not alias handles"
    );
    let stats = world.resource::<UsdTextureCache>().stats();
    assert_eq!(stats.lookups, 2);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.decode_calls, 2);
}

#[test]
fn reused_texture_lookup_does_not_decode_again() {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(UsdTextureCache::default());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/external/franka/panda/DetailedProps/Materials/Textures/normal.png")
        .to_string_lossy()
        .into_owned();

    let first = resolve_texture(&mut world, &path, false).expect("texture loads");
    world.resource_mut::<UsdTextureCache>().reset_stats();
    let reused = resolve_texture(&mut world, &path, false).expect("texture remains cached");

    assert_eq!(first, reused);
    assert_eq!(
        world.resource::<UsdTextureCache>().stats(),
        TextureCacheStats {
            lookups: 1,
            hits: 1,
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
            retired_assets: 1,
            cleaned_assets: 0,
            cleanup_passes: 0,
            cleanup_entities_scanned: 0,
        }
    );
}

#[test]
fn archive_lookup_does_not_discover_unregistered_repository_archives() {
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(UsdTextureCache::default());

    assert!(resolve_texture(&mut world, "unregistered/texture.png", true).is_none());
    let stats = world.resource::<UsdTextureCache>().stats();
    assert_eq!(stats.archive_scans, 0);
    assert_eq!(stats.archive_entries_scanned, 0);
    assert_eq!(stats.archive_index_builds, 0);
    assert_eq!(stats.archive_entries_indexed, 0);
}
