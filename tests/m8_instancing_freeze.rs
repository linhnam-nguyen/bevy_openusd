//! M8-C6+ release benchmark through the real LiveStage reconciliation path.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use bevy::asset::Assets;
use bevy::mesh::{Indices, Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;
use serde::Serialize;
use usd_bevy::read::geom::read_point_instancer;
use usd_bevy::route::cache::ProjectionCache;
use usd_bevy::route::instancer::UsdInstance;
use usd_bevy::{LiveStage, LiveStagePlugin, PointInstancerStats, UsdPlugin};

const FIXTURE: &str = "assets/external/PointInstancedMedCity.usdz";
const INSTANCER: &str = "/MediterraneanHills/Buildings";

#[derive(Debug, Serialize)]
struct M8C6Artifact {
    schema: &'static str,
    checkpoint: &'static str,
    git_sha: String,
    build_profile: &'static str,
    fixture: &'static str,
    logical_instance_count: usize,
    visible_instance_count: usize,
    ecs_entity_count: usize,
    mesh_entity_count: usize,
    unique_mesh_handles: usize,
    mesh_asset_count_before_reproject: usize,
    mesh_asset_count_after_reproject: usize,
    material_asset_count: usize,
    estimated_mesh_cpu_bytes: usize,
    initial_projection_ms: f64,
    live_transform_reproject_ms: f64,
    sparse_transform_patches: u64,
    instance_spawns: u64,
    instance_despawns: u64,
    transform_updates: u64,
    cache_lookups: u64,
    cache_hits: u64,
    cache_misses: u64,
    renderer_extraction_ms: Option<f64>,
    draw_batch_count: Option<usize>,
    gpu_memory_bytes: Option<usize>,
}

fn mesh_bytes(mesh: &Mesh) -> usize {
    let attributes = mesh
        .attributes()
        .map(|(_, values)| values.get_bytes().len())
        .sum::<usize>();
    let indices = match mesh.indices() {
        Some(Indices::U16(values)) => values.len() * std::mem::size_of::<u16>(),
        Some(Indices::U32(values)) => values.len() * std::mem::size_of::<u32>(),
        None => 0,
    };
    attributes + indices
}

fn git_sha() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git is available");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git SHA is UTF-8")
        .trim()
        .to_owned()
}

fn main_metrics(world: &mut World) -> (usize, usize, usize, usize, usize, usize, usize) {
    let mut instances = world.query::<&UsdInstance>();
    let visible = instances.iter(world).count();
    let mut meshes = world.query::<&Mesh3d>();
    let mesh_entities = meshes.iter(world).count();
    let mut entities = world.query::<Entity>();
    let ecs_entities = entities.iter(world).count();
    let mut mesh_handles = HashSet::new();
    let mut render_rows =
        world.query::<(&UsdInstance, &Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    for (_, mesh, _) in render_rows.iter(world) {
        mesh_handles.insert(mesh.0.id());
    }
    let meshes = world.resource::<Assets<Mesh>>();
    let estimated_bytes = meshes.iter().map(|(_, mesh)| mesh_bytes(mesh)).sum();
    let mesh_assets = meshes.iter().count();
    let materials = world.resource::<Assets<StandardMaterial>>().iter().count();
    (
        visible,
        ecs_entities,
        mesh_entities,
        mesh_handles.len(),
        mesh_assets,
        materials,
        estimated_bytes,
    )
}

#[test]
fn records_m8_c6_instancing_freeze() {
    assert!(!cfg!(debug_assertions), "M8-C6+ requires a release build");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let stage = Stage::open(fixture_path.to_str().expect("fixture path is valid"))
        .expect("PointInstancedMedCity opens");
    let instancer_path = Path::new(INSTANCER).expect("instancer path is valid");
    let logical_instance_count = read_point_instancer(&stage, &instancer_path)
        .expect("PointInstancer read succeeds")
        .expect("PointInstancer exists")
        .positions
        .len();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(LiveStage::new(stage.clone()));
    let projection_started = Instant::now();
    app.update();
    let initial_projection_ms = projection_started.elapsed().as_secs_f64() * 1000.0;

    let (_, _, _, _, mesh_assets_before, _, _) = main_metrics(app.world_mut());
    let (
        visible,
        ecs_entity_count,
        mesh_entity_count,
        unique_mesh_handles,
        _,
        material_asset_count,
        estimated_mesh_cpu_bytes,
    ) = main_metrics(app.world_mut());
    *app.world_mut().resource_mut::<PointInstancerStats>() = PointInstancerStats::default();

    let updated = read_point_instancer(
        &app.world()
            .get_non_send::<LiveStage>()
            .expect("live stage exists")
            .stage,
        &instancer_path,
    )
    .expect("PointInstancer reread succeeds")
    .expect("PointInstancer remains available")
    .positions
    .into_iter()
    .enumerate()
    .map(|(index, mut position)| {
        if index == 0 {
            position[0] += 0.25;
        }
        Vec3f::from(position)
    })
    .collect();
    app.world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists")
        .stage
        .prim(instancer_path)
        .attribute("positions")
        .set(Value::Vec3fVec(updated))
        .expect("live transform edit succeeds");
    let started = Instant::now();
    app.update();
    let live_transform_reproject_ms = started.elapsed().as_secs_f64() * 1000.0;
    let stats = *app.world().resource::<PointInstancerStats>();
    let (_, _, _, _, mesh_assets_after, _, _) = main_metrics(app.world_mut());

    let cache = app.world().resource::<ProjectionCache>().stats();
    let artifact = M8C6Artifact {
        schema: "usdhub.m8.c6.instancing-freeze.v2",
        checkpoint: "M8-C6+",
        git_sha: git_sha(),
        build_profile: "release",
        fixture: FIXTURE,
        logical_instance_count,
        visible_instance_count: visible,
        ecs_entity_count,
        mesh_entity_count,
        unique_mesh_handles,
        mesh_asset_count_before_reproject: mesh_assets_before,
        mesh_asset_count_after_reproject: mesh_assets_after,
        material_asset_count,
        estimated_mesh_cpu_bytes,
        initial_projection_ms,
        live_transform_reproject_ms,
        sparse_transform_patches: stats.sparse_transform_patches,
        instance_spawns: stats.instance_spawns,
        instance_despawns: stats.instance_despawns,
        transform_updates: stats.transform_updates,
        cache_lookups: cache.lookups,
        cache_hits: cache.hits,
        cache_misses: cache.misses,
        renderer_extraction_ms: None,
        draw_batch_count: None,
        gpu_memory_bytes: None,
    };
    assert_eq!(logical_instance_count, visible);
    assert_eq!(mesh_assets_before, mesh_assets_after);
    assert_eq!(unique_mesh_handles, 8);
    assert!(live_transform_reproject_ms > 0.0);
    assert_eq!(stats.sparse_transform_patches, 1);
    assert_eq!(stats.instance_spawns, 0);
    assert_eq!(stats.instance_despawns, 0);
    assert_eq!(stats.transform_updates, visible as u64);

    let artifact_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/m8-c6-instancing-freeze.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("benchmark serializes"),
    )
    .expect("benchmark artifact writes");
    println!("M8-C6+ artifact: {}", artifact_path.display());
}
