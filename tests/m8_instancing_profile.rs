//! M8-C1 release baseline for repeated geometry and PointInstancer projection.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use bevy::asset::Assets;
use bevy::mesh::{Indices, Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use openusd::sdf::Path;
use openusd::usd::Stage;
use serde::Serialize;
use usd_bevy::read::geom::read_point_instancer;
use usd_bevy::route::instancer::UsdInstance;
use usd_bevy::{LiveStage, PrimEntities, UsdPlugin, project_stage};

#[derive(Debug, Serialize)]
struct M8C1Artifact {
    schema: &'static str,
    checkpoint: &'static str,
    git_sha: String,
    build_profile: &'static str,
    fixture: &'static str,
    point_instancer_prim_count: usize,
    logical_instance_count: usize,
    visible_instance_count: usize,
    ecs_entity_count: usize,
    mesh_entity_count: usize,
    unique_mesh_handles: usize,
    mesh_asset_count: usize,
    unique_material_handles: usize,
    material_asset_count: usize,
    estimated_mesh_cpu_bytes: usize,
    projection_ms: f64,
    renderer_extraction_ms: Option<f64>,
    draw_batch_count: Option<usize>,
    gpu_memory_bytes: Option<usize>,
}

fn benchmark_git_sha() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git is available for benchmark provenance");
    assert!(output.status.success(), "git rev-parse HEAD succeeds");
    String::from_utf8(output.stdout)
        .expect("git SHA is UTF-8")
        .trim()
        .to_owned()
}

fn logical_instance_count(stage: &Stage, path: &Path) -> usize {
    read_point_instancer(stage, path)
        .expect("PointInstancer read succeeds")
        .expect("fixture contains the expected PointInstancer")
        .positions
        .len()
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

#[test]
fn records_m8_c1_point_instancer_baseline() {
    assert!(!cfg!(debug_assertions), "M8-C1 requires a release build");
    let fixture = "assets/external/PointInstancedMedCity.usdz";
    let stage_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture);
    let stage = Stage::open(stage_path.to_str().expect("fixture path is valid"))
        .expect("PointInstancedMedCity opens");
    let instancer_path = Path::new("/MediterraneanHills/Buildings").expect("valid instancer path");
    let logical_instance_count = logical_instance_count(&stage, &instancer_path);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(UsdPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));

    let projection_started = Instant::now();
    let mut prim_entities = PrimEntities::default();
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    project_stage(app.world_mut(), &live, &mut prim_entities);
    app.world_mut().insert_non_send(live);
    let projection_ms = projection_started.elapsed().as_secs_f64() * 1000.0;

    let world = app.world_mut();
    let mut instance_query = world.query::<&UsdInstance>();
    let visible_instance_count = instance_query.iter(world).count();
    let mut mesh_query = world.query::<&Mesh3d>();
    let mesh_entity_count = mesh_query.iter(world).count();
    let mut entity_query = world.query::<Entity>();
    let ecs_entity_count = entity_query.iter(world).count();
    let mut mesh_handles = HashSet::new();
    let mut material_handles = HashSet::new();
    let mut mesh_entity_query = world.query::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    for (mesh, material) in mesh_entity_query.iter(world) {
        mesh_handles.insert(mesh.0.id());
        material_handles.insert(material.0.id());
    }
    let meshes = world.resource::<Assets<Mesh>>();
    let estimated_mesh_cpu_bytes = meshes.iter().map(|(_, mesh)| mesh_bytes(mesh)).sum();
    let artifact = M8C1Artifact {
        schema: "usdhub.m8.c1.instancing-baseline.v1",
        checkpoint: "M8-C1",
        git_sha: benchmark_git_sha(),
        build_profile: "release",
        fixture,
        point_instancer_prim_count: 1,
        logical_instance_count,
        visible_instance_count,
        ecs_entity_count,
        mesh_entity_count,
        unique_mesh_handles: mesh_handles.len(),
        mesh_asset_count: meshes.iter().count(),
        unique_material_handles: material_handles.len(),
        material_asset_count: world.resource::<Assets<StandardMaterial>>().iter().count(),
        estimated_mesh_cpu_bytes,
        projection_ms,
        renderer_extraction_ms: None,
        draw_batch_count: None,
        gpu_memory_bytes: None,
    };
    assert_eq!(
        artifact.logical_instance_count,
        artifact.visible_instance_count
    );
    assert!(artifact.visible_instance_count > 0);
    assert!(artifact.unique_mesh_handles > 0);
    assert!(artifact.projection_ms > 0.0);

    let artifact_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/m8-c1-instancing-baseline.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("benchmark serializes"),
    )
    .expect("benchmark artifact writes");
    println!("M8-C1 artifact: {}", artifact_path.display());
}
