//! Shared Bevy viewport test support: proven app builders and entity queries.

use bevy::prelude::*;

/// Create the asset-enabled USD app used by semantic viewport tests.
pub fn semantic_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app
}

/// Create a live test app projecting a pre-opened OpenUSD stage.
pub fn live_test_app(stage: openusd::usd::Stage) -> App {
    let mut app = App::new();
    app.add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage));
    app.update();
    app
}

/// Query an entity by USD prim path from the PrimEntities resource.
pub fn projected_entity(app: &App, path: &str) -> Entity {
    let world = app.world();
    world
        .resource::<usd_bevy::PrimEntities>()
        .entity(world.resource::<usd_bevy::PathStore>(), path)
        .unwrap_or_else(|| panic!("{path} entity exists"))
}

/// Return sorted prim paths currently mapped in PrimEntities.
pub fn sorted_paths(map: &usd_bevy::PrimEntities, paths: &usd_bevy::PathStore) -> Vec<String> {
    let mut paths = map
        .iter(paths)
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[cfg(test)]
#[path = "tests/unit/self_tests.rs"]
mod tests;
