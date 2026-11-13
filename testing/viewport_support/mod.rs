//! Shared public-API setup for viewport test apps.

use bevy::prelude::*;

/// Create the asset-enabled USD app used by semantic viewport tests.
pub(super) fn semantic_test_app() -> App {
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
