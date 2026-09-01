use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::StandardMaterial;
use bevy::prelude::{App, GlobalTransform, Vec3};
use openusd::usd::Stage;

use crate::UsdPlugin;
use crate::live::{LiveStage, LiveStagePlugin, PathStore, PrimEntities};

fn characterization_stage() -> Stage {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages/native_instance_characterization.usda");
    Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("native instance fixture opens")
}

fn projected_entity(app: &App, path: &str) -> bevy::prelude::Entity {
    let world = app.world();
    world
        .resource::<PrimEntities>()
        .entity(world.resource::<PathStore>(), path)
        .unwrap_or_else(|| panic!("{path} entity exists"))
}

#[test]
fn native_instance_global_transforms_include_distinct_instance_roots() {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(LiveStage::new(characterization_stage()));
    app.update();

    let frame_a = projected_entity(&app, "/World/Window_A/Frame");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");
    let global_a = app
        .world()
        .get::<GlobalTransform>(frame_a)
        .expect("frame A global transform")
        .compute_transform();
    let global_b = app
        .world()
        .get::<GlobalTransform>(frame_b)
        .expect("frame B global transform")
        .compute_transform();
    assert_eq!(global_a.translation, Vec3::new(-3.0, 0.0, 0.0));
    assert_eq!(global_b.translation, Vec3::new(3.0, 0.0, 0.0));
    assert_ne!(global_a.translation, global_b.translation);
    assert_eq!(
        app.world().get::<Mesh3d>(frame_a).expect("frame A mesh").0,
        app.world().get::<Mesh3d>(frame_b).expect("frame B mesh").0,
        "distinct instance transforms still use one shared mesh handle"
    );
}
