use bevy::prelude::*;
use openusd::usd::Stage;
use std::time::Duration;

use crate::live::{
    LiveStage, LiveStagePlugin, PrimEntities, ProjectionPlan, ProjectionReadiness, project_stage,
};
use crate::prim_ref::UsdPrimRef;
use crate::snippet::UsdSnippet;

fn hierarchy_stage() -> openusd::usd::Stage {
    UsdSnippet::new(
        r#"#usda 1.0

def Xform "Z"
{
    def Xform "Child"
    {
    }
}
def Xform "A"
{
    def Xform "Leaf"
    {
    }
}
def Xform "B"
{
}
"#,
    )
    .open_stage()
    .expect("hierarchy stage opens")
}

fn animated_stage() -> Stage {
    Stage::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/stages/animated_translate.usda")
            .to_str()
            .expect("animated fixture path is valid"),
    )
    .expect("animated stage opens")
}

fn sorted_paths(map: &PrimEntities) -> Vec<String> {
    let mut paths = map
        .iter()
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn unlimited_progressive_queue_matches_direct_projection() {
    let direct_stage = animated_stage();
    let queued_stage = animated_stage();
    let direct_live = LiveStage::new(direct_stage);
    let queued_live = LiveStage::new(queued_stage);

    let mut direct_world = World::new();
    let mut direct_map = PrimEntities::default();
    project_stage(&mut direct_world, &direct_live, &mut direct_map);

    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(queued_live);
    app.update();

    let queued_map = app.world().resource::<PrimEntities>();
    assert_eq!(sorted_paths(queued_map), sorted_paths(&direct_map));
    assert_eq!(
        &app.world().resource::<crate::live::AnimatedPrims>().0,
        &direct_world.resource::<crate::live::AnimatedPrims>().0
    );
    assert_eq!(
        app.world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready
    );
    for path in sorted_paths(queued_map) {
        let queued_entity = queued_map.entity(&path).unwrap();
        let direct_entity = direct_map.entity(&path).unwrap();
        assert_eq!(
            app.world().get::<UsdPrimRef>(queued_entity),
            direct_world.get::<UsdPrimRef>(direct_entity)
        );
        assert_eq!(
            app.world().get::<Transform>(queued_entity),
            direct_world.get::<Transform>(direct_entity)
        );
        assert_eq!(
            app.world().get::<Visibility>(queued_entity),
            direct_world.get::<Visibility>(direct_entity)
        );
    }
}

#[test]
fn ready_queue_short_circuits_without_rebuilding_the_plan() {
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let first = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .clone();
    app.update();
    let second = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(second.plan_builds(), first.plan_builds());
    assert_eq!(second.resident_short_circuits(), 1);
}

fn run_until_ready(app: &mut App) {
    for _ in 0..32 {
        if app
            .world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness()
            == ProjectionReadiness::Ready
        {
            return;
        }
        app.update();
    }
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    panic!(
        "progressive projection did not become ready: completed={} total={} readiness={:?}",
        state.completed(),
        state.total(),
        state.readiness()
    );
}

#[test]
fn work_budget_projects_one_entry_per_update_and_reaches_final_equality() {
    let stage = hierarchy_stage();
    let expected = ProjectionPlan::from_stage(&stage).expect("expected plan builds");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(crate::live::ProjectionBudget::work_items(1));
    app.world_mut().insert_non_send(LiveStage::new(stage));

    app.update();
    let partial = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(partial.completed(), 1, "one root entry is projected first");
    assert_eq!(partial.readiness(), ProjectionReadiness::Planning);
    assert_eq!(app.world().resource::<PrimEntities>().len(), 1);

    run_until_ready(&mut app);
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(state.completed(), expected.len());
    assert_eq!(state.progress(), 1.0);
    assert_eq!(app.world().resource::<PrimEntities>().len(), expected.len());
}

#[test]
fn explicit_time_budget_can_yield_without_consuming_work() {
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(crate::live::ProjectionBudget::time(Duration::ZERO));
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(state.completed(), 0);
    assert_eq!(state.readiness(), ProjectionReadiness::Planning);
}

#[test]
fn cancellation_discards_partial_generation_and_restarts() {
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(crate::live::ProjectionBudget::work_items(1));
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let old_generation = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .generation();
    app.world_mut()
        .resource_mut::<crate::live::ProgressiveProjectionState>()
        .cancel();
    assert_eq!(
        app.world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Cancelled
    );

    app.update();
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(state.generation(), old_generation + 1);
    assert_eq!(state.cancelled_generations(), 1);
    assert_eq!(state.completed(), 1);
    run_until_ready(&mut app);
}

#[test]
fn reload_midway_cancels_old_session_and_never_mixes_paths() {
    let replacement = UsdSnippet::new(
        r#"#usda 1.0
def Xform "Reloaded"
{
    def Xform "Child"
    {
    }
}
"#,
    )
    .open_stage()
    .expect("replacement stage opens");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(crate::live::ProjectionBudget::work_items(1));
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let old_session = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .session_id();

    app.world_mut().insert_non_send(LiveStage::new(replacement));
    app.update();
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_ne!(state.session_id(), old_session);
    assert_eq!(state.completed(), 1);
    assert_eq!(app.world().resource::<PrimEntities>().entity("/A"), None);
    assert!(app.world().resource::<PrimEntities>().entity("/").is_some());

    run_until_ready(&mut app);
    let paths = sorted_paths(app.world().resource::<PrimEntities>());
    assert_eq!(paths, vec!["/", "/Reloaded", "/Reloaded/Child"]);
}

#[test]
fn readiness_is_additive_and_progress_is_monotonic() {
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(crate::live::ProjectionBudget::work_items(1));
    assert_eq!(
        app.world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Idle
    );
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let projecting = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(projecting.readiness(), ProjectionReadiness::Planning);
    assert_eq!(projecting.progress(), 0.0);
    let first_progress = projecting.progress();

    app.update();
    let second_progress = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .progress();
    assert!(second_progress >= first_progress);
    run_until_ready(&mut app);
    let mut ready = app
        .world_mut()
        .resource_mut::<crate::live::ProgressiveProjectionState>();
    assert_eq!(ready.readiness(), ProjectionReadiness::Ready);
    assert_eq!(ready.progress(), 1.0);
    ready.cancel();
    assert_eq!(ready.readiness(), ProjectionReadiness::Ready);
}

#[test]
fn cache_aware_gate_rebuilds_when_a_resident_mesh_asset_is_evicted() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
def Cube "Cube"
{
}
"#,
    )
    .open_stage()
    .expect("cube stage opens");
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<bevy::image::Image>>()
        .init_resource::<Assets<bevy::pbr::StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    let cube = app
        .world()
        .resource::<PrimEntities>()
        .entity("/Cube")
        .unwrap();
    let mesh = app
        .world()
        .get::<Mesh3d>(cube)
        .expect("cube mesh")
        .0
        .clone();
    let plan_builds = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .plan_builds();
    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .remove(mesh.id());
    app.world_mut()
        .resource_mut::<crate::live::ProgressiveProjectionState>()
        .invalidate_resident_cache();

    app.update();
    let state = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(state.readiness(), ProjectionReadiness::Ready);
    assert_eq!(state.plan_builds(), plan_builds + 1);
    let rebuilt_entity = app
        .world()
        .resource::<PrimEntities>()
        .entity("/Cube")
        .expect("rebuilt cube entity");
    let rebuilt = app
        .world()
        .get::<Mesh3d>(rebuilt_entity)
        .expect("rebuilt cube mesh")
        .0
        .clone();
    assert!(app.world().resource::<Assets<Mesh>>().contains(&rebuilt));
}
