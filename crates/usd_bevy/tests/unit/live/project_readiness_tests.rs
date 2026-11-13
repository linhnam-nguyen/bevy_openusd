use super::*;
use crate::{LiveStage, LiveStagePlugin, ProjectionBudget, UsdPlugin, UsdSnippet};
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;

fn geometry_stage() -> openusd::usd::Stage {
    UsdSnippet::new(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/stages/live/project_readiness_tests_1.usda"
    )))
    .open_stage()
    .expect("geometry stage opens")
}

#[test]
fn readiness_advances_through_existing_renderer_milestones() {
    let mut state = ProjectOpenReadinessState::default();

    assert!(state.observe(11, 1, ProjectionReadiness::Planning, false));
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::StageOpened));
    assert!(state.observe(11, 1, ProjectionReadiness::Projecting, true));
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::FirstGeometry));
    assert!(state.observe(11, 1, ProjectionReadiness::Ready, true));
    assert_eq!(
        state.readiness(),
        Some(ProjectOpenReadiness::ProjectionComplete)
    );
}

#[test]
fn a_replaced_session_ignores_old_projection_completion() {
    let mut state = ProjectOpenReadinessState::default();
    state.observe(11, 1, ProjectionReadiness::Ready, true);
    assert_eq!(state.activation_generation(), 1);

    state.observe(22, 2, ProjectionReadiness::Planning, false);
    assert_eq!(state.activation_generation(), 2);
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::StageOpened));

    assert!(!state.observe(11, 1, ProjectionReadiness::Ready, true));
    assert_eq!(state.stage_session_id(), Some(22));
    assert_eq!(state.projection_generation(), 2);
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::StageOpened));
}

#[test]
fn an_older_projection_generation_cannot_regress_current_session() {
    let mut state = ProjectOpenReadinessState::default();
    state.observe(11, 4, ProjectionReadiness::Projecting, true);
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::FirstGeometry));

    assert!(!state.observe(11, 3, ProjectionReadiness::Ready, true));
    assert_eq!(state.projection_generation(), 4);
    assert_eq!(state.readiness(), Some(ProjectOpenReadiness::FirstGeometry));
}

#[test]
fn live_stage_plugin_observes_readiness_without_a_second_projection_scheduler() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>();
    app.insert_resource(ProjectionBudget::work_items(1));
    app.world_mut()
        .insert_non_send(LiveStage::new(geometry_stage()));

    app.update();
    let first = app.world().resource::<ProjectOpenReadinessState>();
    assert_eq!(first.readiness(), Some(ProjectOpenReadiness::StageOpened));
    let projection_generation = app
        .world()
        .resource::<ProgressiveProjectionState>()
        .generation();
    assert_eq!(first.projection_generation(), projection_generation);

    let mut saw_first_geometry = false;
    for _ in 0..64 {
        app.update();
        let readiness = app
            .world()
            .resource::<ProjectOpenReadinessState>()
            .readiness();
        saw_first_geometry |= readiness == Some(ProjectOpenReadiness::FirstGeometry);
        if readiness == Some(ProjectOpenReadiness::ProjectionComplete) {
            break;
        }
    }

    let readiness = app.world().resource::<ProjectOpenReadinessState>();
    assert!(saw_first_geometry);
    assert_eq!(
        readiness.readiness(),
        Some(ProjectOpenReadiness::ProjectionComplete)
    );
    assert_eq!(
        app.world()
            .resource::<ProgressiveProjectionState>()
            .plan_builds(),
        1
    );
}
