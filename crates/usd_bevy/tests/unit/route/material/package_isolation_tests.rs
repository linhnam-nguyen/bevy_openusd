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
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<super::UsdMaterialCache>();
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

    // F-C03-I1-01: Also verify relative inner path resolution (`textures/albedo.png`
    // without `pkg[inner]` prefix) through the candidate `for usdz in &usdz_files`
    // loop across distinct packages A and B, missing inner entry isolation, and
    // same-filename archive replacement invalidating the cached entry bytes.
    world
        .resource_mut::<UsdTextureCache>()
        .replace_active_archives([package_a.clone()]);
    let rel_a = resolve_texture(&mut world, "textures/albedo.png", true)
        .expect("candidate A relative lookup succeeds");
    assert_eq!(first_pixel(&world, &rel_a), [255, 0, 0, 255]);
    let descriptor = crate::read::shade::ReadPreviewMaterial {
        diffuse_texture: Some("textures/albedo.png".to_owned()),
        ..Default::default()
    };
    let material_a =
        super::material_cache::intern_material(&mut world, "/Material", &descriptor).unwrap();

    assert!(
        resolve_texture(&mut world, "textures/missing.png", true).is_none(),
        "missing inner entry in candidate A returns None without reading unrelated bytes"
    );

    world
        .resource_mut::<UsdTextureCache>()
        .replace_active_archives([package_b.clone()]);
    let rel_b = resolve_texture(&mut world, "textures/albedo.png", true)
        .expect("candidate B relative lookup succeeds");
    assert_eq!(first_pixel(&world, &rel_b), [0, 0, 255, 255]);
    let material_b =
        super::material_cache::intern_material(&mut world, "/Material", &descriptor).unwrap();
    assert_ne!(
        material_a, material_b,
        "equal authored descriptors can resolve different stage texture bytes"
    );
    let texture = world
        .resource::<Assets<StandardMaterial>>()
        .get(&material_b)
        .unwrap()
        .base_color_texture
        .as_ref()
        .unwrap();
    assert_eq!(first_pixel(&world, texture), [0, 0, 255, 255]);

    // Replace package A at the same filename with new pixel bytes (and append an extra
    // entry so its file length / ArchiveFingerprint changes reliably even within the
    // same filesystem timestamp tick).
    {
        let file = std::fs::File::create(&package_a).expect("replace USDZ package A");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("textures/albedo.png", options)
            .expect("create updated package texture entry");
        archive
            .write_all(&png_pixel([0, 255, 0, 255]))
            .expect("write updated package texture entry");
        archive
            .start_file("scene.usda", options)
            .expect("create scene entry");
        archive
            .write_all(b"#usda 1.0\n")
            .expect("write scene entry");
        archive.finish().expect("finish replaced USDZ package A");
    }
    world
        .resource_mut::<UsdTextureCache>()
        .replace_active_archives([package_a.clone()]);
    let rel_a_replaced = resolve_texture(&mut world, "textures/albedo.png", true)
        .expect("replaced candidate A relative lookup succeeds");
    assert_eq!(first_pixel(&world, &rel_a_replaced), [0, 255, 0, 255]);
}

#[test]
fn active_stage_archive_ownership_is_composition_scoped() {
    let source = usd_support::resolve_asset_path("external/hummingbird.usdz");
    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(source.to_str().expect("fixture path is valid"))
        .expect("Hummingbird stage opens");
    let archives = super::archive_paths_for_stage(&stage, &source).expect("archive closure");
    assert_eq!(archives, vec![std::fs::canonicalize(source).unwrap()]);
}

#[test]
fn hummingbird_materials_use_authored_conversion_and_embedded_textures() {
    let source = usd_support::resolve_asset_path("external/hummingbird.usdz");
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(source.to_str().expect("fixture path is valid"))
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

#[test]
fn generated_usdz_decodes_embedded_1x1_texture() {
    let dir = tempdir().expect("tempdir");
    let archive = dir.path().join("red_box.usdz");
    let usda = r#"#usda 1.0
(
    defaultPrim = "World"
    upAxis = "Y"
    metersPerUnit = 1.0
)

def Xform "World"
{
    def Scope "Materials"
    {
        def Material "Red"
        {
            token outputs:surface.connect = </World/Materials/Red/Surface.outputs:surface>

            def Shader "Surface"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor.connect = </World/Materials/Red/DiffuseTex.outputs:rgb>
                float inputs:roughness = 0.5
                token outputs:surface
            }

            def Shader "DiffuseTex"
            {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @./textures/red.png@
                token inputs:sourceColorSpace = "sRGB"
                float3 outputs:rgb
            }
        }
    }

    def Cube "RedBox" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {
        double size = 0.5
        rel material:binding = </World/Materials/Red>
    }
}
"#;
    {
        let file = std::fs::File::create(&archive).expect("create red_box.usdz");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("scene.usda", options).unwrap();
        writer.write_all(usda.as_bytes()).unwrap();
        writer.start_file("textures/red.png", options).unwrap();
        writer.write_all(&png_pixel([255, 0, 0, 255])).unwrap();
        writer.finish().unwrap();
    }

    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(archive.to_str().expect("archive path"))
        .expect("usdz opens");
    let archives = super::archive_paths_for_stage(&stage, &archive).expect("archive closure");
    let mut app = bevy::app::App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(UsdTextureCache {
            archive_paths: archives,
            ..Default::default()
        });
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mat_handle = world
        .query::<(&UsdPrimRef, &MeshMaterial3d<StandardMaterial>)>()
        .iter(world)
        .find(|(prim, _)| prim.path == "/World/RedBox")
        .map(|(_, mat)| mat.0.clone())
        .expect("RedBox material projected");
    let mat = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&mat_handle)
        .expect("RedBox StandardMaterial exists");
    let tex = mat
        .base_color_texture
        .as_ref()
        .expect("base_color_texture decoded from USDZ");
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(tex)
        .expect("embedded 1x1 PNG image exists");
    assert_eq!(image.texture_descriptor.size.width, 1);
    assert_eq!(image.texture_descriptor.size.height, 1);
}
