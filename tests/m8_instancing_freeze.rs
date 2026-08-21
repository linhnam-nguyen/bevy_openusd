//! M8-C6 release benchmark with cache and live-transform evidence.

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
use usd_bevy::route::instancer::{PointInstancerRoute, UsdInstance};
use usd_bevy::{LiveStage, PrimEntities, PrimRoute, RouteCtx, UsdPlugin, project_stage};

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
    reproject_ms: f64,
    live_transform_reproject_ms: f64,
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

fn project_instances(app: &mut App, stage: &Stage, entity: Entity) -> f64 {
    let path = Path::new(INSTANCER).expect("instancer path is valid");
    let route = PointInstancerRoute;
    let started = Instant::now();
    route.project(&RouteCtx::new(stage, &path), app.world_mut(), entity);
    started.elapsed().as_secs_f64() * 1000.0
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
    assert!(!cfg!(debug_assertions), "M8-C6 requires a release build");
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
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(LiveStage::new(stage.clone()));
    let projection_started = Instant::now();
    let mut map = PrimEntities::default();
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    project_stage(app.world_mut(), &live, &mut map);
    app.world_mut().insert_non_send(live);
    let initial_projection_ms = projection_started.elapsed().as_secs_f64() * 1000.0;
    let entity = map.entity(INSTANCER).expect("instancer entity exists");

    let (_, _, _, _, mesh_assets_before, _, _) = main_metrics(app.world_mut());
    let reproject_ms = project_instances(&mut app, &stage, entity);
    let (
        visible,
        ecs_entities,
        mesh_entities,
        unique_mesh_handles,
        mesh_assets_after,
        materials,
        bytes,
    ) = main_metrics(app.world_mut());

    let mut updated = read_point_instancer(&stage, &instancer_path)
        .expect("PointInstancer reread succeeds")
        .expect("PointInstancer remains available")
        .positions;
    updated[0][0] += 0.25;
    let positions = updated.into_iter().map(Vec3f::from).collect();
    stage
        .prim(instancer_path.clone())
        .attribute("positions")
        .set(Value::Vec3fVec(positions))
        .expect("live transform edit succeeds");
    let live_transform_reproject_ms = project_instances(&mut app, &stage, entity);

    let cache = app.world().resource::<ProjectionCache>().stats();
    let artifact = M8C6Artifact {
        schema: "usdhub.m8.c6.instancing-freeze.v1",
        checkpoint: "M8-C6",
        git_sha: git_sha(),
        build_profile: "release",
        fixture: FIXTURE,
        logical_instance_count,
        visible_instance_count: visible,
        ecs_entity_count: ecs_entities,
        mesh_entity_count: mesh_entities,
        unique_mesh_handles,
        mesh_asset_count_before_reproject: mesh_assets_before,
        mesh_asset_count_after_reproject: mesh_assets_after,
        material_asset_count: materials,
        estimated_mesh_cpu_bytes: bytes,
        initial_projection_ms,
        reproject_ms,
        live_transform_reproject_ms,
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
    assert!(reproject_ms > 0.0);
    assert!(live_transform_reproject_ms > 0.0);

    let artifact_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/m8-c6-instancing-freeze.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("benchmark serializes"),
    )
    .expect("benchmark artifact writes");
    println!("M8-C6 artifact: {}", artifact_path.display());
}
