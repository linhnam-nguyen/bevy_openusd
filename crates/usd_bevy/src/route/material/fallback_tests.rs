use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

use crate::route::material::{
    MaterialProjectionProvenance, MaterialProjectionStatus, UsdMaterialCache,
};
use crate::{LiveStage, LiveStagePlugin, ProjectionSeed, UsdPlugin};

fn build_app_for(fixture: &str) -> App {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages")
        .join(fixture);
    let stage = openusd::usd::Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("material fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

#[test]
fn fallback_material_is_marked_disposable_and_not_in_material_cache() {
    let app = build_app_for("material_fallback.usda");
    let status = app
        .world()
        .resource::<MaterialProjectionProvenance>()
        .status("/World/Broken");
    assert_eq!(status, Some(MaterialProjectionStatus::Fallback));
    assert!(app.world().resource::<UsdMaterialCache>().is_empty());
    assert_eq!(
        app.world().resource::<ProjectionSeed>().pending_materials(),
        0
    );
}
