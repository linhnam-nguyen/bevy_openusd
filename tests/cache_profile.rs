//! Milestone 18 fixture profile for the projected-mesh cache.
//!
//! This target is explicit because the root package intentionally disables
//! automatic integration-test discovery. It measures a real repeated-mesh USD
//! scene and an embedded-texture USDZ before any cache policy change is made.

use std::path::PathBuf;
use std::time::Instant;

use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use openusd::usd::Stage;
use serde::Serialize;
use usd_bevy::{
    LiveStage, LiveStagePlugin, PrimEntities, UsdPlugin, apply_change_batch, project_stage,
};

#[derive(Debug, Serialize)]
struct M6C5Artifact {
    schema: &'static str,
    checkpoint: &'static str,
    fixture: &'static str,
    unique_bindings: usize,
    shared_material_consumers: usize,
    material_assets_before_edit: usize,
    material_assets_after_edit: usize,
    initial_projection_ms: f64,
    live_edit_ms: f64,
    initial_material_lookups: u64,
    initial_material_hits: u64,
    initial_material_misses: u64,
    initial_descriptor_changes: u64,
    live_material_lookups: u64,
    live_material_hits: u64,
    live_material_misses: u64,
    live_descriptor_changes: u64,
    live_retired_assets: u64,
    live_cleaned_assets: u64,
    initial_texture_entries: usize,
    initial_texture_lookups: u64,
    initial_texture_hits: u64,
    initial_texture_misses: u64,
    initial_texture_decode_calls: u64,
    live_texture_lookups: u64,
    live_texture_hits: u64,
    live_texture_misses: u64,
    live_texture_decode_calls: u64,
    live_cleanup_passes: u64,
    live_cleanup_entities_scanned: u64,
}

fn build_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: "tests/stages".into(),
            ..Default::default()
        })
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(UsdPlugin);
    app
}

fn build_live_material_app() -> App {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stages/materials_network.usda");
    let stage = Stage::open(path.to_str().expect("materials fixture path is valid"))
        .expect("materials fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

fn apply_live_material_edit(app: &mut App) -> f64 {
    let started = Instant::now();
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    let batch = live.drain_change_batch().expect("material edit is queued");
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);
    started.elapsed().as_secs_f64() * 1000.0
}

#[test]
fn profiles_repeated_mesh_fixture() {
    let mut app = build_test_app();
    let stage = Stage::open("tests/stages/instanceable.usda").expect("fixture stage opens");
    let live = LiveStage::new(stage);
    let mut map = PrimEntities::default();
    project_stage(app.world_mut(), &live, &mut map);

    let stats = app
        .world()
        .resource::<usd_bevy::route::cache::ProjectionCache>()
        .stats();
    println!(
        "cache profile: lookups={}, hits={}, misses={}, stale_handles={}, evictions={}",
        stats.lookups, stats.hits, stats.misses, stats.stale_handles, stats.evictions
    );

    assert!(stats.lookups > 0, "fixture should project mesh geometry");
    assert!(
        stats.hits > 0,
        "repeated fixture mesh should produce a cache hit"
    );
    assert_eq!(stats.lookups, stats.hits + stats.misses);
    assert_eq!(stats.stale_handles, 0);
    assert_eq!(stats.evictions, 0);
}

#[test]
fn profiles_larger_kitchen_usdz_fixture() {
    let mut app = build_test_app();
    let stage = Stage::open("assets/external/Kitchen_set.usdz").expect("Kitchen_set opens");
    let live = LiveStage::new(stage);
    let mut map = PrimEntities::default();
    project_stage(app.world_mut(), &live, &mut map);

    let stats = app
        .world()
        .resource::<usd_bevy::route::cache::ProjectionCache>()
        .stats();
    println!(
        "Kitchen_set cache profile: lookups={}, hits={}, misses={}, stale_handles={}, evictions={}",
        stats.lookups, stats.hits, stats.misses, stats.stale_handles, stats.evictions
    );
    assert!(
        stats.lookups > 0,
        "Kitchen_set should project mesh geometry"
    );
    assert_eq!(stats.lookups, stats.hits + stats.misses);
    assert_eq!(stats.stale_handles, 0);
    assert_eq!(stats.evictions, 0);
}

#[test]
fn profiles_shared_material_binding_fixture() {
    let mut app = build_test_app();
    let stage = Stage::open("tests/stages/materials.usda").expect("materials fixture opens");
    let live = LiveStage::new(stage);
    let mut map = PrimEntities::default();
    project_stage(app.world_mut(), &live, &mut map);

    let stats = app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    println!(
        "material profile: lookups={}, hits={}, misses={}, stale_handles={}, descriptor_changes={}",
        stats.lookups, stats.hits, stats.misses, stats.stale_handles, stats.descriptor_changes
    );

    assert_eq!(stats.lookups, 4);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.stale_handles, 0);
    assert_eq!(stats.descriptor_changes, 0);
}

#[test]
fn profiles_embedded_texture_usdz_fixture() {
    let mut app = build_test_app();
    let archive = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/external/usdz_sample.usdz")
        .canonicalize()
        .expect("USDZ texture fixture exists");
    app.world_mut()
        .resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        .archive_paths
        .push(archive);

    let stage = Stage::open("assets/external/usdz_sample.usdz").expect("USDZ sample opens");
    let live = LiveStage::new(stage);
    let mut map = PrimEntities::default();
    project_stage(app.world_mut(), &live, &mut map);

    let stats = app
        .world()
        .resource::<usd_bevy::route::material::UsdTextureCache>()
        .stats();
    println!(
        "USDZ texture profile: lookups={}, hits={}, misses={}, stale_handles={}, load_failures={}, archive_scans={}, archive_entries_scanned={}, archive_hits={}, archive_misses={}, archive_index_builds={}, archive_index_invalidations={}, archive_entries_indexed={}",
        stats.lookups,
        stats.hits,
        stats.misses,
        stats.stale_handles,
        stats.load_failures,
        stats.archive_scans,
        stats.archive_entries_scanned,
        stats.archive_hits,
        stats.archive_misses,
        stats.archive_index_builds,
        stats.archive_index_invalidations,
        stats.archive_entries_indexed
    );

    assert_eq!(stats.lookups, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.stale_handles, 0);
    assert_eq!(stats.load_failures, 0);
    assert_eq!(stats.archive_scans, 1);
    assert_eq!(stats.archive_entries_scanned, 2);
    assert_eq!(stats.archive_hits, 1);
    assert_eq!(stats.archive_misses, 0);
    assert_eq!(stats.archive_index_builds, 1);
    assert_eq!(stats.archive_index_invalidations, 0);
    assert_eq!(stats.archive_entries_indexed, 2);
}

#[test]
fn records_m6_shared_material_benchmark_artifact() {
    let mut app = build_live_material_app();
    let initial_projection_ms = app
        .world()
        .resource::<usd_bevy::ProjectionStats>()
        .initial_projection_ms
        .expect("initial projection timing");
    let initial_material = app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    let unique_bindings = app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .len();
    let material_assets_before_edit = app.world().resource::<Assets<StandardMaterial>>().len();
    let initial_texture = app
        .world()
        .resource::<usd_bevy::route::material::UsdTextureCache>()
        .stats();
    let initial_texture_entries = app
        .world()
        .resource::<usd_bevy::route::material::UsdTextureCache>()
        .textures
        .len();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("stage");
        live.stage
            .prim(openusd::sdf::path("/World/SharedShaders/RedAlbedo").unwrap())
            .attribute("inputs:file")
            .set(openusd::sdf::Value::AssetPath(
                "assets/external/franka/panda/DetailedProps/Materials/Textures/Logo_Textures_Albedo.png"
                    .into(),
            ))
            .expect("texture edit authors");
    }
    app.world_mut()
        .resource_mut::<usd_bevy::route::material::UsdMaterialCache>()
        .reset_stats();
    app.world_mut()
        .resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        .reset_stats();
    let live_edit_ms = apply_live_material_edit(&mut app);
    let live_material = app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    let live_texture = app
        .world()
        .resource::<usd_bevy::route::material::UsdTextureCache>()
        .stats();
    let material_assets_after_edit = app.world().resource::<Assets<StandardMaterial>>().len();

    let artifact = M6C5Artifact {
        schema: "usdhub.m6.c5.shared-material.v2",
        checkpoint: "M6-C5",
        fixture: "tests/stages/materials_network.usda",
        unique_bindings,
        shared_material_consumers: 2,
        material_assets_before_edit,
        material_assets_after_edit,
        initial_projection_ms,
        live_edit_ms,
        initial_material_lookups: initial_material.lookups,
        initial_material_hits: initial_material.hits,
        initial_material_misses: initial_material.misses,
        initial_descriptor_changes: initial_material.descriptor_changes,
        live_material_lookups: live_material.lookups,
        live_material_hits: live_material.hits,
        live_material_misses: live_material.misses,
        live_descriptor_changes: live_material.descriptor_changes,
        live_retired_assets: live_material.retired_assets,
        live_cleaned_assets: live_material.cleaned_assets,
        initial_texture_entries,
        initial_texture_lookups: initial_texture.lookups,
        initial_texture_hits: initial_texture.hits,
        initial_texture_misses: initial_texture.misses,
        initial_texture_decode_calls: initial_texture.decode_calls,
        live_texture_lookups: live_texture.lookups,
        live_texture_hits: live_texture.hits,
        live_texture_misses: live_texture.misses,
        live_texture_decode_calls: live_texture.decode_calls,
        live_cleanup_passes: live_material.cleanup_passes,
        live_cleanup_entities_scanned: live_material.cleanup_entities_scanned,
    };
    let artifact_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/m6-c5-shared-material.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("M6 artifact serializes"),
    )
    .expect("M6 artifact writes");
    assert_eq!(unique_bindings, 3);
    assert_eq!(material_assets_before_edit, 3);
    assert_eq!(material_assets_after_edit, material_assets_before_edit);
    assert_eq!(initial_texture_entries, 1);
    assert_eq!(initial_texture.lookups, 1);
    assert_eq!(initial_texture.hits, 0);
    assert_eq!(initial_texture.misses, 1);
    assert_eq!(initial_texture.decode_calls, 1);
    assert_eq!(live_material.lookups, 2);
    assert_eq!(live_material.hits, 1);
    assert_eq!(live_material.misses, 1);
    assert_eq!(live_material.descriptor_changes, 1);
    assert_eq!(live_material.retired_assets, live_material.cleaned_assets);
    assert_eq!(live_material.cleanup_passes, 1);
    assert_eq!(live_material.cleanup_entities_scanned, 4);
    assert_eq!(live_texture.lookups, 1);
    assert_eq!(live_texture.hits, 0);
    assert_eq!(live_texture.misses, 1);
    assert_eq!(live_texture.decode_calls, 1);
    println!(
        "M6-C5 shared-material artifact: {}",
        artifact_path.display()
    );
}
