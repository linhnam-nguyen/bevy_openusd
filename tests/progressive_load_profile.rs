//! M7-C6 release benchmark for progressive initial projection.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use bevy::image::Image;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use openusd::usd::Stage;
use serde::Serialize;
use usd_bevy::{
    LiveStage, LiveStagePlugin, ProgressiveProjectionState, ProjectionBudget, ProjectionReadiness,
    UsdPlugin,
};

#[derive(Debug, Serialize)]
struct M7C6Artifact {
    schema: &'static str,
    checkpoint: &'static str,
    git_sha: String,
    build_profile: &'static str,
    fixture: &'static str,
    stage_open_ms: f64,
    first_projected_prim_ms: f64,
    first_mesh_ms: f64,
    first_geometry_frame_ms: f64,
    progress_25_ms: f64,
    progress_50_ms: f64,
    progress_75_ms: f64,
    progress_100_ms: f64,
    total_projection_ms: f64,
    p95_update_ms: f64,
    longest_main_thread_stall_ms: f64,
    loading_updates: usize,
    total_prims: usize,
    plan_builds: u64,
    resident_short_circuits: u64,
}

fn benchmark_git_sha() -> String {
    let output = Command::new("git")
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

fn percentile_95(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let index = ((sorted.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

fn benchmark_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_resource(ProjectionBudget::work_items(64));
    app
}

#[test]
fn records_m7_progressive_load_benchmark_artifact() {
    assert!(!cfg!(debug_assertions), "M7-C6 requires a release build");
    let fixture = "assets/external/Kitchen_set.usdz";
    let open_started = Instant::now();
    let stage_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture);
    let stage = Stage::open(stage_path.to_str().expect("Kitchen path is valid"))
        .expect("Kitchen_set opens");
    let stage_open_ms = open_started.elapsed().as_secs_f64() * 1000.0;

    let mut app = benchmark_app();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    let projection_started = Instant::now();
    let mut update_samples = Vec::new();
    let mut milestones = [None; 4];
    let mut first_geometry_frame_ms = None;
    let loading_updates = loop {
        assert!(update_samples.len() < 10_000, "projection did not finish");
        let update_started = Instant::now();
        app.update();
        let update_ms = update_started.elapsed().as_secs_f64() * 1000.0;
        update_samples.push(update_ms);
        let elapsed_ms = projection_started.elapsed().as_secs_f64() * 1000.0;
        let (readiness, completed, total) = {
            let state = app.world().resource::<ProgressiveProjectionState>();
            (state.readiness(), state.completed(), state.total())
        };
        let progress = completed as f64 / total.max(1) as f64;
        for (index, threshold) in [0.25, 0.50, 0.75, 1.0].into_iter().enumerate() {
            if milestones[index].is_none() && progress >= threshold {
                milestones[index] = Some(elapsed_ms);
            }
        }
        let has_geometry = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<Mesh3d>>();
            query.iter(world).next().is_some()
        };
        if first_geometry_frame_ms.is_none() && has_geometry {
            first_geometry_frame_ms = Some(elapsed_ms);
        }
        if readiness == ProjectionReadiness::Ready {
            break update_samples.len();
        }
    };

    let state = app.world().resource::<ProgressiveProjectionState>();
    let artifact = M7C6Artifact {
        schema: "usdhub.m7.c6.progressive-load.v1",
        checkpoint: "M7-C6",
        git_sha: benchmark_git_sha(),
        build_profile: "release",
        fixture,
        stage_open_ms,
        first_projected_prim_ms: state.first_projected_prim_ms().unwrap_or_default(),
        first_mesh_ms: state.first_mesh_ms().unwrap_or_default(),
        first_geometry_frame_ms: first_geometry_frame_ms.unwrap_or_default(),
        progress_25_ms: milestones[0].unwrap_or_default(),
        progress_50_ms: milestones[1].unwrap_or_default(),
        progress_75_ms: milestones[2].unwrap_or_default(),
        progress_100_ms: milestones[3].unwrap_or_default(),
        total_projection_ms: projection_started.elapsed().as_secs_f64() * 1000.0,
        p95_update_ms: percentile_95(&update_samples),
        longest_main_thread_stall_ms: update_samples.iter().copied().fold(0.0, f64::max),
        loading_updates,
        total_prims: state.completed().saturating_sub(1),
        plan_builds: state.plan_builds(),
        resident_short_circuits: state.resident_short_circuits(),
    };
    assert!(artifact.first_projected_prim_ms > 0.0);
    assert!(artifact.first_mesh_ms > 0.0);
    assert!(artifact.first_geometry_frame_ms > 0.0);
    assert!(artifact.progress_100_ms > 0.0);
    let artifact_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/m7-progressive-load.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("benchmark serializes"),
    )
    .expect("benchmark artifact writes");
    println!("M7-C6 artifact: {}", artifact_path.display());
}
