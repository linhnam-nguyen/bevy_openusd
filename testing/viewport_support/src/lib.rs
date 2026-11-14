//! Shared Bevy Viewport test support: test App builders, entity query helpers, and harness runners.

use bevy::prelude::*;

/// Representation of a USD asset for test loaders and harness runners.
#[derive(bevy::reflect::TypePath, bevy::asset::Asset, Debug, Clone)]
pub struct UsdAsset {
    pub scene: Handle<bevy::scene::ScenePatch>,
}

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

/// Create a test app configured with an asset directory and full USD/render pipelines.
pub fn build_test_app(fixture_dir: &str) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: fixture_dir.into(),
            ..Default::default()
        })
        .init_asset::<bevy::scene::ScenePatch>()
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(bevy::scene::ScenePlugin)
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin)
        .register_type::<bevy::mesh::Mesh3d>()
        .register_type::<bevy::pbr::MeshMaterial3d<StandardMaterial>>();
    app
}

/// Create a live test app projecting a pre-opened OpenUSD stage.
pub fn live_test_app(stage: openusd::usd::Stage) -> App {
    let mut app = App::new();
    app.add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage));
    app.update();
    app
}

/// Load a USD asset by name and drive the app until the asset state is Loaded.
pub fn load_and_drive(app: &mut App, asset_name: &str) -> Handle<UsdAsset> {
    use bevy::asset::{AssetServer, LoadState};
    let handle: Handle<UsdAsset> = app
        .world()
        .resource::<AssetServer>()
        .load(asset_name.to_string());
    for _ in 0..200 {
        app.update();
        let server = app.world().resource::<AssetServer>();
        match server.get_load_state(&handle) {
            Some(LoadState::Loaded) => return handle,
            Some(LoadState::Failed(err)) => panic!("UsdAsset load failed: {err}"),
            _ => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    panic!("UsdAsset did not load in time");
}

/// Spawn a scene root from a loaded UsdAsset handle and tick the app to settle.
pub fn spawn_scene_root(app: &mut App, handle: &Handle<UsdAsset>) -> Entity {
    use bevy::scene::ScenePatchInstance;
    let scene_handle = {
        let assets = app.world().resource::<Assets<UsdAsset>>();
        assets.get(handle).expect("asset missing").scene.clone()
    };
    let entity = app.world_mut().spawn(ScenePatchInstance(scene_handle)).id();
    for _ in 0..10 {
        app.update();
    }
    entity
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
mod tests {
    use super::*;

    #[test]
    fn test_semantic_test_app_initialization() {
        let mut app = semantic_test_app();
        app.update();
        assert!(app.world().contains_resource::<usd_bevy::PrimEntities>());
        assert!(app.world().contains_resource::<usd_bevy::PathStore>());
    }

    #[test]
    fn test_sorted_paths() {
        let mut paths = usd_bevy::PathStore::default();
        let mut map = usd_bevy::PrimEntities::default();
        let mut app = App::new();
        let e1 = app.world_mut().spawn_empty().id();
        let e2 = app.world_mut().spawn_empty().id();
        let e3 = app.world_mut().spawn_empty().id();
        map.insert(&mut paths, "/World/Z", e1);
        map.insert(&mut paths, "/World/A", e2);
        map.insert(&mut paths, "/World/M", e3);

        let sorted = sorted_paths(&map, &paths);
        assert_eq!(sorted, vec!["/World/A", "/World/M", "/World/Z"]);
    }
}
