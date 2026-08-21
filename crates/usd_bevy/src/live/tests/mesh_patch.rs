use bevy::asset::Assets;
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::Value;
use serde::Serialize;
use std::time::Instant;

use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PrimEntities, ReconcileStats, StageChange,
    StageChangeBatch, apply_change_batch,
};
use crate::{GeometryProfile, UsdPlugin, UsdSnippet};

const MESH_PATH: &str = "/World/Triangle";

#[derive(Serialize)]
struct LivePatchSample {
    operation: String,
    patch_latency_ms: f64,
    mesh_route_samples: usize,
    mesh_conversions: usize,
    source_cache_lookups: usize,
    source_cache_hits: usize,
    source_cache_misses: usize,
    final_cache_hits: usize,
    final_cache_misses: usize,
    reconcile_visited_stage_prims: usize,
    reconcile_patched_entities: usize,
    reconcile_spawned_entities: usize,
    reconcile_despawned_entities: usize,
}

#[derive(Serialize)]
struct M5C4Artifact {
    schema: &'static str,
    checkpoint: &'static str,
    fixture: &'static str,
    initial: crate::route::profile::GeometryProfileTotals,
    operations: Vec<LivePatchSample>,
    final_totals: crate::route::profile::GeometryProfileTotals,
    fallback_material_artifact: &'static str,
}

fn mesh_stage() -> openusd::usd::Stage {
    UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def Mesh "Triangle"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        color3f[] primvars:displayColor = [(1, 0, 0), (0, 1, 0), (0, 0, 1)] (
            interpolation = "vertex"
        )
        float3[] extent = [(0, 0, 0), (1, 1, 0)]
    }
}
"#,
    )
    .open_stage()
    .expect("live mesh patch stage opens")
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin);
    app.add_plugins(LiveStagePlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.world_mut().resource_mut::<GeometryProfile>().enabled = true;
    app.world_mut()
        .insert_non_send(LiveStage::new(mesh_stage()));
    app.update();
    app
}

fn mesh_handle(app: &App) -> Handle<Mesh> {
    let entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(MESH_PATH)
        .expect("triangle entity exists");
    app.world()
        .get::<Mesh3d>(entity)
        .expect("triangle mesh exists")
        .0
        .clone()
}

fn mesh_totals(app: &App) -> crate::route::profile::GeometryProfileTotals {
    app.world().resource::<GeometryProfile>().totals
}

fn delta(after: usize, before: usize) -> usize {
    after.saturating_sub(before)
}

fn record_operation<F>(
    app: &mut App,
    name: &str,
    before: crate::route::profile::GeometryProfileTotals,
    operation: F,
) -> LivePatchSample
where
    F: FnOnce(&mut App) -> f64,
{
    let patch_latency_ms = operation(app);
    let after = mesh_totals(app);
    let reconcile = *app.world().resource::<ReconcileStats>();
    LivePatchSample {
        operation: name.to_string(),
        patch_latency_ms,
        mesh_route_samples: delta(after.mesh_route_samples, before.mesh_route_samples),
        mesh_conversions: delta(after.mesh_count, before.mesh_count),
        source_cache_lookups: delta(after.source_cache_lookups, before.source_cache_lookups),
        source_cache_hits: delta(after.source_cache_hits, before.source_cache_hits),
        source_cache_misses: delta(after.source_cache_misses, before.source_cache_misses),
        final_cache_hits: delta(after.cache_hits, before.cache_hits),
        final_cache_misses: delta(after.cache_misses, before.cache_misses),
        reconcile_visited_stage_prims: reconcile.visited_stage_prims,
        reconcile_patched_entities: reconcile.patched_entities,
        reconcile_spawned_entities: reconcile.spawned_entities,
        reconcile_despawned_entities: reconcile.despawned_entities,
    }
}

fn apply_changed_info(app: &mut App, property: &str) -> f64 {
    let started = Instant::now();
    apply_batch(
        app,
        StageChangeBatch {
            revision: LiveRevision(100),
            changes: vec![StageChange {
                resynced: Vec::new(),
                changed_info: vec![format!("{MESH_PATH}.{property}")],
            }],
        },
    );
    started.elapsed().as_secs_f64() * 1000.0
}

fn apply_batch(app: &mut App, batch: StageChangeBatch) {
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);
}

fn set_points(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let path = openusd::sdf::path(MESH_PATH).expect("mesh path is valid");
    live.stage
        .prim(path)
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([2.0, 0.0, 0.0]),
            Vec3f::from([0.0, 2.0, 0.0]),
        ]))
        .expect("points authoring succeeds");
    let _ = live.drain_change_batch();
}

fn set_display_color(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let path = openusd::sdf::path(MESH_PATH).expect("mesh path is valid");
    live.stage
        .prim(path)
        .attribute("primvars:displayColor")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 1.0, 1.0]),
            Vec3f::from([1.0, 1.0, 0.0]),
            Vec3f::from([1.0, 0.0, 1.0]),
        ]))
        .expect("display color authoring succeeds");
    let _ = live.drain_change_batch();
}

fn add_subtree_mesh(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let mesh = live.stage.define_prim("/World/NewTriangle").unwrap();
    mesh.create_attribute("faceVertexCounts", "int[]")
        .unwrap()
        .set(Value::IntVec(vec![3]))
        .unwrap();
    mesh.create_attribute("faceVertexIndices", "int[]")
        .unwrap()
        .set(Value::IntVec(vec![0, 1, 2]))
        .unwrap();
    mesh.create_attribute("points", "point3f[]")
        .unwrap()
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([1.0, 0.0, 0.0]),
            Vec3f::from([0.0, 1.0, 0.0]),
        ]))
        .unwrap();
    mesh.create_attribute("primvars:displayColor", "color3f[]")
        .unwrap()
        .set_metadata("interpolation", Value::Token("vertex".into()))
        .unwrap()
        .set(Value::Vec3fVec(vec![
            Vec3f::from([1.0, 0.0, 0.0]),
            Vec3f::from([0.0, 1.0, 0.0]),
            Vec3f::from([0.0, 0.0, 1.0]),
        ]))
        .unwrap();
    let _ = live.drain_change_batch();
}

#[test]
fn m5_c4_live_patch_matrix_keeps_unrelated_edits_out_of_mesh_conversion() {
    let mut app = build_app();
    let initial_handle = mesh_handle(&app);
    let initial = mesh_totals(&app);
    assert_eq!(initial.mesh_count, 1, "initial mesh is one conversion");
    assert_eq!(initial.mesh_route_samples, 1);
    assert_eq!(initial.source_cache_misses, 1);
    assert_eq!(initial.cache_misses, 1);
    let mut samples = Vec::new();

    for property in [
        "xformOp:translate",
        "visibility",
        "material:binding",
        "kind",
    ] {
        let before = mesh_totals(&app);
        let sample = record_operation(&mut app, property, before, |app| {
            apply_changed_info(app, property)
        });
        assert_eq!(sample.mesh_conversions, 0, "{property} must not convert");
        assert_eq!(sample.source_cache_lookups, 0, "{property} skips mesh work");
        assert_eq!(
            mesh_handle(&app),
            initial_handle,
            "{property} keeps mesh handle"
        );
        samples.push(sample);
    }

    set_points(&mut app);
    let before_points = mesh_totals(&app);
    let points_sample = record_operation(&mut app, "points", before_points, |app| {
        apply_changed_info(app, "points")
    });
    assert_eq!(points_sample.mesh_conversions, 1);
    assert_eq!(points_sample.source_cache_misses, 1);
    assert_eq!(points_sample.final_cache_misses, 1);
    assert_ne!(mesh_handle(&app), initial_handle, "points replace the mesh");
    samples.push(points_sample);

    set_display_color(&mut app);
    let before_primvar = mesh_totals(&app);
    let primvar_sample =
        record_operation(&mut app, "primvars:displayColor", before_primvar, |app| {
            apply_changed_info(app, "primvars:displayColor")
        });
    assert_eq!(primvar_sample.mesh_conversions, 1);
    assert_eq!(primvar_sample.source_cache_misses, 1);
    assert_eq!(primvar_sample.final_cache_misses, 1);
    samples.push(primvar_sample);

    let before_add = mesh_totals(&app);
    let add_sample = record_operation(&mut app, "subtree-add", before_add, |app| {
        add_subtree_mesh(app);
        let started = Instant::now();
        apply_batch(
            app,
            StageChangeBatch {
                revision: LiveRevision(101),
                changes: vec![StageChange {
                    resynced: vec!["/World/NewTriangle".to_string()],
                    changed_info: Vec::new(),
                }],
            },
        );
        started.elapsed().as_secs_f64() * 1000.0
    });
    assert_eq!(add_sample.mesh_conversions, 0);
    assert_eq!(add_sample.source_cache_hits, 1);
    assert_eq!(add_sample.final_cache_hits, 0);
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity("/World/NewTriangle")
            .is_some(),
        "subtree add is projected"
    );
    samples.push(add_sample);

    let before_remove = mesh_totals(&app);
    let remove_sample = record_operation(&mut app, "subtree-remove", before_remove, |app| {
        let live = app
            .world()
            .get_non_send::<LiveStage>()
            .expect("live stage exists");
        live.stage.remove_prim("/World/NewTriangle").unwrap();
        let _ = live.drain_change_batch();
        live.enqueue_resync("/World/NewTriangle");
        let started = Instant::now();
        app.update();
        started.elapsed().as_secs_f64() * 1000.0
    });
    assert_eq!(remove_sample.mesh_conversions, 0);
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity("/World/NewTriangle")
            .is_none(),
        "subtree removal is reconciled"
    );
    samples.push(remove_sample);

    let final_totals = mesh_totals(&app);
    let artifact = M5C4Artifact {
        schema: "usdhub.m5.c4.live-mesh-patch.v1",
        checkpoint: "M5-C4+",
        fixture: "inline-single-mesh-with-subtree-edit",
        initial,
        operations: samples,
        final_totals,
        fallback_material_artifact: "target/m5-c3-fallback-material.json",
    };
    let artifact_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/m5-c4-live-mesh-patch.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("live matrix artifact serializes"),
    )
    .expect("live matrix artifact writes");

    println!(
        "M5-C4 live patch matrix: initial_conversions={} final_conversions={} source_hits={} final_cache_hits={} artifact={}",
        initial.mesh_count,
        final_totals.mesh_count,
        final_totals.source_cache_hits,
        final_totals.cache_hits,
        artifact_path.display()
    );
}
