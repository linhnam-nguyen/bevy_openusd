use bevy::prelude::*;

use crate::live::{
    LiveStage, LiveStagePlugin, PrimEntities, ProjectionBudget, ProjectionReadiness,
};
use crate::snippet::UsdSnippet;

fn hierarchy_stage() -> openusd::usd::Stage {
    UsdSnippet::new(
        r#"#usda 1.0
def Xform "A"
{
    def Xform "Child"
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

fn sorted_paths(map: &PrimEntities) -> Vec<String> {
    let mut paths = map
        .iter()
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn same_session_stage_change_restarts_before_stale_projection_continues() {
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut()
        .insert_resource(ProjectionBudget::work_items(1));
    app.world_mut()
        .insert_non_send(LiveStage::new(hierarchy_stage()));
    app.update();
    let old_generation = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>()
        .generation();

    app.world_mut()
        .get_non_send_mut::<LiveStage>()
        .expect("live stage exists")
        .stage
        .define_prim("/A/Changed")
        .expect("same-session prim add succeeds");
    app.update();
    assert_eq!(
        app.world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .generation(),
        old_generation
    );

    app.update();
    let restarted = app
        .world()
        .resource::<crate::live::ProgressiveProjectionState>();
    assert_eq!(restarted.generation(), old_generation + 1);
    assert_eq!(restarted.readiness(), ProjectionReadiness::Planning);

    for _ in 0..128 {
        if app
            .world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness()
            == ProjectionReadiness::Ready
        {
            break;
        }
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<crate::live::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready
    );
    assert_eq!(
        sorted_paths(app.world().resource::<PrimEntities>()),
        vec!["/", "/A", "/A/Changed", "/A/Child", "/B"]
    );
}
