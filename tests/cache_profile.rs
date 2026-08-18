//! Milestone 18 fixture profile for the projected-mesh cache.
//!
//! This target is explicit because the root package intentionally disables
//! automatic integration-test discovery. It measures a real repeated-mesh USD
//! scene before any cache policy change is made.

use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveStage, PrimEntities, UsdPlugin, project_stage};

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
