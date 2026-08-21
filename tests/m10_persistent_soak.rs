//! M10-C4 persistent Bevy runtime/cache soak.
//!
//! One process owns one Bevy `App` for every cycle. The harness deliberately
//! reloads stages, applies live transform/visibility/material/geometry and
//! subtree edits, reprojects a PointInstancer, and replaces a render-target
//! image generation without rebuilding the application.

use std::path::PathBuf;

use bevy::asset::Assets;
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use openusd::gf::Vec3f;
use openusd::sdf::Value;
use openusd::usd::Stage;
use serde::Serialize;
use usd_bevy::route::cache::ProjectionCache;
use usd_bevy::route::material::{UsdMaterialCache, UsdTextureCache};
use usd_bevy::{
    LiveStage, LiveStagePlugin, PointInstancerStats, PrimEntities, ProjectionBudget,
    ProjectionReadiness, UsdPlugin, author_transform,
};

const DEFAULT_CYCLES: usize = 12;

#[derive(Debug, Serialize)]
struct CycleSample {
    cycle: usize,
    fixture: String,
    session_id: u64,
    resize_generation: u64,
    resize_width: u32,
    resize_height: u32,
    projected_prims: usize,
    mesh_assets: usize,
    material_assets: usize,
    image_assets: usize,
    projection_cache_meshes: usize,
    projection_cache_sources: usize,
    material_cache_entries: usize,
    texture_cache_entries: usize,
    point_instancer_full_projects: u64,
    point_instancer_sparse_transform_patches: u64,
    point_instancer_spawns: u64,
    point_instancer_despawns: u64,
    projection_ms: Option<f64>,
}

#[derive(Debug, Serialize)]
struct BoundSummary {
    metric: &'static str,
    all_cycles_min: usize,
    all_cycles_max: usize,
    steady_cycles_min: usize,
    steady_cycles_max: usize,
    bounded: bool,
}

#[derive(Debug, Serialize)]
struct PersistentSoakArtifact {
    schema: &'static str,
    checkpoint: &'static str,
    build_profile: &'static str,
    process_id: u32,
    cycle_count: usize,
    persistent_app: bool,
    workload_sequence: Vec<&'static str>,
    bounds: Vec<BoundSummary>,
    samples: Vec<CycleSample>,
    passed: bool,
}

fn root_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn open_fixture(relative: &str) -> Stage {
    let path = root_path(relative);
    Stage::open(path.to_str().expect("fixture path is valid")).expect("soak fixture opens")
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_resource(ProjectionBudget::unlimited());
    app
}

fn settle_projection(app: &mut App) {
    for _ in 0..256 {
        app.update();
        let state = app
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>();
        match state.readiness() {
            ProjectionReadiness::Ready => return,
            ProjectionReadiness::Failed => panic!("persistent soak projection failed"),
            ProjectionReadiness::Idle
            | ProjectionReadiness::Planning
            | ProjectionReadiness::Projecting
            | ProjectionReadiness::Cancelled => {}
        }
    }
    panic!("persistent soak projection did not settle");
}

fn replace_stage(app: &mut App, fixture: &str) {
    app.world_mut()
        .insert_non_send(LiveStage::new(open_fixture(fixture)));
    settle_projection(app);
}

fn apply_material_network_edits(app: &mut App, cycle: usize) {
    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        author_transform(
            &live.stage,
            "/World/RedBox",
            &Transform::from_translation(Vec3::new(cycle as f32, 0.25, 0.0)),
        )
        .expect("transform edit authors");
    }
    app.update();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage
            .prim(openusd::sdf::path("/World/RedBox").expect("red box path"))
            .create_attribute("visibility", "token")
            .expect("visibility attribute creates")
            .set(Value::Token(if cycle.is_multiple_of(2) {
                "invisible".into()
            } else {
                "inherited".into()
            }))
            .expect("visibility edit authors");
    }
    app.update();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage
            .prim(openusd::sdf::path("/World/RedBox").expect("red box path"))
            .relationship("material:binding")
            .set_targets([openusd::sdf::path(if cycle.is_multiple_of(2) {
                "/World/Materials/GreenMetal"
            } else {
                "/World/Materials/Red"
            })
            .expect("material path")])
            .expect("material edit authors");
    }
    app.update();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage
            .prim(openusd::sdf::path("/World/RedBox").expect("red box path"))
            .attribute("size")
            .set(Value::Double(0.5 + cycle as f64 * 0.01))
            .expect("geometry edit authors");
    }
    app.update();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage
            .define_prim("/World/M10SoakMarker")
            .expect("subtree edit adds marker");
    }
    app.update();
    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage
            .remove_prim("/World/M10SoakMarker")
            .expect("subtree edit removes marker");
    }
    app.update();
}

fn apply_instanceable_edit(app: &mut App, cycle: usize) {
    let live = app.world().get_non_send::<LiveStage>().expect("live stage");
    author_transform(
        &live.stage,
        "/World/InstanceA",
        &Transform::from_translation(Vec3::new(-1.5 + cycle as f32 * 0.05, 0.0, 0.0)),
    )
    .expect("instanceable transform edit authors");
    app.update();
}

fn apply_point_instancer_edit(app: &mut App, cycle: usize) {
    let live = app.world().get_non_send::<LiveStage>().expect("live stage");
    let offset = cycle as f32 * 0.1;
    live.stage
        .prim(openusd::sdf::path("/World/Instances").expect("instancer path"))
        .attribute("positions")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([offset, 0.0, 0.0]),
            Vec3f::from([2.0 + offset, 0.0, 0.0]),
            Vec3f::from([4.0 + offset, 0.0, 0.0]),
            Vec3f::from([6.0 + offset, 0.0, 0.0]),
            Vec3f::from([8.0 + offset, 0.0, 0.0]),
            Vec3f::from([10.0 + offset, 0.0, 0.0]),
        ]))
        .expect("PointInstancer edit authors");
    app.update();
}

fn resize_generation(
    app: &mut App,
    previous: &mut Option<Handle<Image>>,
    cycle: usize,
) -> (u64, u32, u32) {
    if let Some(previous) = previous.take() {
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .remove(previous.id());
    }
    let (width, height) = match cycle % 3 {
        0 => (1280, 720),
        1 => (1920, 1080),
        _ => (2560, 1440),
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            width,
            height,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    *previous = Some(handle);
    (cycle as u64 + 1, width, height)
}

fn sample(
    app: &App,
    cycle: usize,
    fixture: &str,
    resize_generation: u64,
    resize_width: u32,
    resize_height: u32,
) -> CycleSample {
    let projection = app
        .world()
        .resource::<usd_bevy::ProgressiveProjectionState>();
    let instancer = app.world().resource::<PointInstancerStats>();
    CycleSample {
        cycle,
        fixture: fixture.to_owned(),
        session_id: app
            .world()
            .get_non_send::<LiveStage>()
            .expect("live stage")
            .session_id(),
        resize_generation,
        resize_width,
        resize_height,
        projected_prims: app.world().resource::<PrimEntities>().len(),
        mesh_assets: app.world().resource::<Assets<Mesh>>().len(),
        material_assets: app.world().resource::<Assets<StandardMaterial>>().len(),
        image_assets: app.world().resource::<Assets<Image>>().len(),
        projection_cache_meshes: app.world().resource::<ProjectionCache>().len(),
        projection_cache_sources: app.world().resource::<ProjectionCache>().source_len(),
        material_cache_entries: app.world().resource::<UsdMaterialCache>().len(),
        texture_cache_entries: app.world().resource::<UsdTextureCache>().textures.len(),
        point_instancer_full_projects: instancer.full_projects,
        point_instancer_sparse_transform_patches: instancer.sparse_transform_patches,
        point_instancer_spawns: instancer.instance_spawns,
        point_instancer_despawns: instancer.instance_despawns,
        projection_ms: projection
            .plan_complete_ms()
            .or(projection.first_projected_prim_ms()),
    }
}

fn bound(
    samples: &[CycleSample],
    metric: &'static str,
    value: impl Fn(&CycleSample) -> usize,
) -> BoundSummary {
    let steady_start = samples.len() / 2;
    let (all_cycles_min, all_cycles_max) = samples
        .iter()
        .map(&value)
        .fold((usize::MAX, 0), |(minimum, maximum), current| {
            (minimum.min(current), maximum.max(current))
        });
    let (steady_min, steady_max) = samples
        .iter()
        .skip(steady_start)
        .map(&value)
        .fold((usize::MAX, 0), |(minimum, maximum), current| {
            (minimum.min(current), maximum.max(current))
        });
    BoundSummary {
        metric,
        all_cycles_min,
        all_cycles_max,
        steady_cycles_min: steady_min,
        steady_cycles_max: steady_max,
        bounded: steady_max.saturating_sub(steady_min) <= 16,
    }
}

#[test]
fn records_m10_c4_persistent_runtime_soak() {
    if cfg!(debug_assertions) {
        eprintln!("M10-C4 persistent release soak skipped in a debug test build");
        return;
    }
    let cycles = std::env::var("USDHUB_M10_C4_CYCLES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CYCLES);
    assert!(cycles >= 12, "persistent soak needs at least twelve cycles");

    let workloads = [
        "load/reload",
        "transform",
        "visibility",
        "material",
        "geometry",
        "subtree/full-reconcile",
        "PointInstancer reprojection",
        "resize generations",
    ];
    let fixtures = [
        "tests/stages/materials_network.usda",
        "tests/stages/instanceable.usda",
        "tests/stages/m8_point_instancer.usda",
        "assets/external/Kitchen_set.usdz",
    ];
    let mut app = build_app();
    let mut image_handle = None;
    let mut samples = Vec::with_capacity(cycles);

    for cycle in 0..cycles {
        let fixture = fixtures[cycle % fixtures.len()];
        replace_stage(&mut app, fixture);
        if fixture.ends_with("materials_network.usda") {
            apply_material_network_edits(&mut app, cycle);
        } else if fixture.ends_with("instanceable.usda") {
            apply_instanceable_edit(&mut app, cycle);
        } else if fixture.ends_with("m8_point_instancer.usda") {
            apply_point_instancer_edit(&mut app, cycle);
        }
        let (resize_generation, width, height) =
            resize_generation(&mut app, &mut image_handle, cycle);
        app.update();
        samples.push(sample(
            &app,
            cycle + 1,
            fixture,
            resize_generation,
            width,
            height,
        ));
    }

    let bounds = vec![
        bound(&samples, "mesh_assets", |sample| sample.mesh_assets),
        bound(&samples, "material_assets", |sample| sample.material_assets),
        bound(&samples, "image_assets", |sample| sample.image_assets),
        bound(&samples, "projection_cache_meshes", |sample| {
            sample.projection_cache_meshes
        }),
        bound(&samples, "projection_cache_sources", |sample| {
            sample.projection_cache_sources
        }),
        bound(&samples, "material_cache_entries", |sample| {
            sample.material_cache_entries
        }),
        bound(&samples, "texture_cache_entries", |sample| {
            sample.texture_cache_entries
        }),
    ];
    assert!(bounds.iter().all(|summary| summary.bounded));
    assert!(samples.iter().all(|sample| sample.resize_generation > 0));
    assert!(
        samples
            .iter()
            .any(|sample| sample.point_instancer_full_projects > 0)
    );
    assert!(samples.iter().any(|sample| sample.mesh_assets > 0));

    let artifact = PersistentSoakArtifact {
        schema: "usdhub.m10.c4.persistent-soak.v2",
        checkpoint: "M10-C4+",
        build_profile: "release",
        process_id: std::process::id(),
        cycle_count: cycles,
        persistent_app: true,
        workload_sequence: workloads.to_vec(),
        bounds,
        samples,
        passed: true,
    };
    let artifact_path = root_path("target/benchmark/m10-c4-persistent-runtime.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("persistent soak serializes"),
    )
    .expect("persistent soak artifact writes");
    println!(
        "M10-C4 persistent runtime artifact: {}",
        artifact_path.display()
    );
}
