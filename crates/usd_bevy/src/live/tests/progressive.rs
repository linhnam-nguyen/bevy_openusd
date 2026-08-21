use bevy::prelude::*;
use openusd::usd::Stage;

use crate::live::{
    LiveStage, LiveStagePlugin, PrimEntities, ProjectionPlan, ProjectionReadiness,
    collect_stage_subtree_paths, project_stage,
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

#[test]
fn projection_plan_is_deterministic_and_parent_before_child() {
    let stage = hierarchy_stage();
    let first = ProjectionPlan::from_stage(&stage).expect("first plan builds");
    let second = ProjectionPlan::from_stage(&stage).expect("second plan builds");
    assert_eq!(first, second);
    assert_eq!(
        first.paths().collect::<Vec<_>>(),
        vec!["/", "/A", "/B", "/Z", "/A/Leaf", "/Z/Child"]
    );
    for (index, entry) in first.entries().enumerate() {
        if let Some(parent) = entry.parent_index() {
            assert!(parent < index, "parent must precede {}", entry.path());
            assert_eq!(
                first.entry(parent).expect("parent entry").path(),
                match entry.path() {
                    "/A" | "/B" | "/Z" => "/",
                    "/A/Leaf" => "/A",
                    "/Z/Child" => "/Z",
                    path => panic!("unexpected path {path}"),
                }
            );
        }
    }
}

#[test]
fn projection_plan_matches_the_canonical_traversal_predicate() {
    let stage = hierarchy_stage();
    let plan = ProjectionPlan::from_stage(&stage).expect("plan builds");
    let mut traversed = collect_stage_subtree_paths(&stage, "/").expect("paths collect");
    traversed.sort();
    let mut planned = plan.paths().skip(1).map(str::to_owned).collect::<Vec<_>>();
    planned.sort();
    assert_eq!(planned, traversed);
}

#[test]
fn projection_plan_keeps_unloaded_payload_prim_as_placeholder_work() {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def "PayloadPrim" (
        payload = @./sub.usda@</Sub>
    )
    {
    }
}
"#,
    )
    .open_stage()
    .expect("payload stage opens");
    stage.unload(openusd::sdf::path("/World/PayloadPrim").expect("payload prim path"));
    let plan = ProjectionPlan::from_stage(&stage).expect("payload plan builds");
    assert!(plan.paths().any(|path| path == "/World/PayloadPrim"));
    assert!(
        !stage
            .prim(openusd::sdf::path("/World/PayloadPrim").unwrap())
            .is_loaded()
            .expect("payload load state is readable")
    );
}

#[test]
fn subtree_plan_preserves_root_and_parent_relation() {
    let stage = hierarchy_stage();
    let plan = ProjectionPlan::from_subtree(&stage, "/A").expect("subtree plan builds");
    assert_eq!(plan.paths().collect::<Vec<_>>(), vec!["/", "/A", "/A/Leaf"]);
    assert_eq!(plan.entry(1).unwrap().parent_index(), Some(0));
    assert_eq!(plan.entry(2).unwrap().parent_index(), Some(1));
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
