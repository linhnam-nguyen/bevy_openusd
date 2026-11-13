use std::io::{Cursor, Write};

use bevy::image::Image;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{Assets, World};
use image::{ImageFormat, Rgba, RgbaImage};
use openusd::usd::Stage;
use tempfile::tempdir;

use super::UsdTextureCache;
use super::texture_cache::resolve_texture;
use crate::{LiveStage, LiveStagePlugin, UsdPlugin, UsdPrimRef};

fn png_pixel(pixel: [u8; 4]) -> Vec<u8> {
    let mut image = RgbaImage::new(1, 1);
    image.put_pixel(0, 0, Rgba(pixel));
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("encode package texture");
    bytes
}

fn write_package(path: &std::path::Path, pixel: [u8; 4]) {
    let file = std::fs::File::create(path).expect("create USDZ package");
    let mut archive = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive
        .start_file("textures/albedo.png", options)
        .expect("create package texture entry");
    archive
        .write_all(&png_pixel(pixel))
        .expect("write package texture entry");
    archive.finish().expect("finish USDZ package");
}

fn first_pixel(world: &World, handle: &bevy::asset::Handle<Image>) -> [u8; 4] {
    world
        .resource::<Assets<Image>>()
        .get(handle)
        .and_then(|image| image.data.as_ref())
        .map(|data| [data[0], data[1], data[2], data[3]])
        .expect("decoded package image data")
}

#[test]
fn package_texture_identity_survives_a_b_a_activation_sequence() {
    let directory = tempdir().expect("temporary package directory");
    let package_a = directory.path().join("A.usdz");
    let package_b = directory.path().join("B.usdz");
    write_package(&package_a, [255, 0, 0, 255]);
    write_package(&package_b, [0, 0, 255, 255]);

    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(UsdTextureCache::default());

    let activate = |world: &mut World, package: &std::path::Path, expected| {
        world
            .resource_mut::<UsdTextureCache>()
            .replace_active_archives([package.to_path_buf()]);
        let identifier = format!("{}[textures/albedo.png]", package.display());
        let handle = resolve_texture(world, &identifier, true).expect("package texture loads");
        assert_eq!(first_pixel(world, &handle), expected);
    };

    activate(&mut world, &package_a, [255, 0, 0, 255]);
    activate(&mut world, &package_b, [0, 0, 255, 255]);
    activate(&mut world, &package_a, [255, 0, 0, 255]);

    assert_eq!(world.resource::<UsdTextureCache>().textures.len(), 1);
    assert_eq!(world.resource::<UsdTextureCache>().stats().archive_scans, 3);
}

#[test]
fn active_stage_archive_ownership_is_composition_scoped() {
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/external/hummingbird.usdz");
    let stage = Stage::open(source.to_str().expect("fixture path is valid"))
        .expect("Hummingbird stage opens");
    let archives = super::archive_paths_for_stage(&stage, &source).expect("archive closure");
    assert_eq!(archives, vec![std::fs::canonicalize(source).unwrap()]);
}

#[test]
fn hummingbird_materials_use_authored_conversion_and_embedded_textures() {
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/external/hummingbird.usdz");
    let stage = openusd::usd::Stage::open(source.to_str().expect("fixture path is valid"))
        .expect("Hummingbird stage opens");
    let archives = super::archive_paths_for_stage(&stage, &source).expect("archive closure");
    let mut app = bevy::app::App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .insert_resource(UsdTextureCache {
            archive_paths: archives,
            ..Default::default()
        });
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let authored = {
        let world = app.world_mut();
        let provenance = world
            .resource::<super::MaterialProjectionProvenance>()
            .clone();
        let mut query = world.query::<(&UsdPrimRef, &MeshMaterial3d<StandardMaterial>)>();
        query
            .iter(world)
            .filter(|(prim, _)| {
                provenance.status(&prim.path)
                    == Some(super::MaterialProjectionStatus::AuthoredConversion)
            })
            .count()
    };
    assert!(
        authored > 0,
        "Hummingbird has authored material conversions"
    );
    assert!(
        app.world()
            .resource::<Assets<Image>>()
            .iter()
            .next()
            .is_some(),
        "Hummingbird embedded texture is decoded"
    );
    assert_eq!(
        app.world()
            .resource::<UsdTextureCache>()
            .stats()
            .archive_misses,
        0
    );
}
