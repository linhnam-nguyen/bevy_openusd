use std::path::PathBuf;

use bevy::prelude::*;
use usd_bevy::{LiveStagePlugin, UsdPlugin};

use super::*;
use crate::api::{ViewportBridgePlugin, ViewportEventOutbox, ViewportReadModelState};
use crate::app::launch_options::parse_launch_options;
use crate::viewport::stage::{RequestedAsset, StageInfo};

fn minimal_host_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        bevy::asset::AssetPlugin::default(),
    ));
    app.init_resource::<Assets<Shader>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<crate::viewport::rendering::scene::SectionClipMaterial>>();
    app
}

#[test]
fn generic_viewport_wires_without_project_git_turso_or_cache_providers() {
    let mut app = minimal_host_app();

    let config = GenericViewportConfig {
        asset_path: String::new(),
        asset_root: PathBuf::from("assets"),
        fps: 60,
        benchmark_mesh_profile: false,
        skip_initial_stage_load: true,
    };

    configure_generic_viewport_core(&mut app, &config);
    // Calling twice must remain idempotent with unique plugin registration.
    configure_generic_viewport_core(&mut app, &config);

    assert!(app.is_plugin_added::<UsdPlugin>());
    assert!(app.is_plugin_added::<LiveStagePlugin>());
    assert!(!app.is_plugin_added::<ViewportBridgePlugin>());
    assert!(app.world().contains_resource::<StageInfo>());
    assert!(app.world().contains_resource::<RequestedAsset>());
    assert!(app.world().contains_resource::<ViewportEventOutbox>());
    assert!(app.world().contains_resource::<ViewportReadModelState>());
    // Project/Git/Turso/Cache runtime providers must NOT be registered in generic mode.
    assert!(
        !app.world()
            .contains_resource::<crate::semantic::viewport::SemanticSyncRuntimeResource>()
    );
    assert!(
        !app.world()
            .contains_resource::<crate::document::stages::recovery::RecoverySettings>()
    );
    assert!(
        !app.world()
            .contains_resource::<crate::cache::historical_geometry::HistoricalGeometryCache>()
    );

    app.update();
}

#[test]
fn shared_application_composition_preserves_launch_flags_and_injects_product_services() {
    let options = parse_launch_options([
        "--headless".to_string(),
        "--fps".to_string(),
        "30".to_string(),
        "--width".to_string(),
        "1280".to_string(),
        "--height".to_string(),
        "720".to_string(),
    ])
    .expect("valid headless launch options");

    let mut app = minimal_host_app();

    let generic_config = GenericViewportConfig {
        asset_path: String::new(),
        asset_root: PathBuf::from("assets"),
        fps: options.fps,
        benchmark_mesh_profile: options.benchmark_mesh_profile,
        skip_initial_stage_load: true,
    };
    configure_generic_viewport_core(&mut app, &generic_config);
    configure_product_services(&mut app, &options, "", true);

    assert!(app.is_plugin_added::<ViewportBridgePlugin>());
    assert!(
        app.world()
            .contains_resource::<crate::document::stages::recovery::RecoverySettings>()
    );
    assert_eq!(
        app.world()
            .resource::<RendererCadence>()
            .effective_renderer_target_fps(),
        Some(30)
    );

    app.update();
    let mut outbox = app.world_mut().resource_mut::<ViewportEventOutbox>();
    let mut saw_ready = false;
    while let Some(envelope) = outbox.pop() {
        if matches!(envelope.event, viewport_protocol::ViewportEvent::Ready { .. }) {
            saw_ready = true;
        }
    }
    assert!(saw_ready, "ViewportBridgePlugin emits protocol Ready on Startup");
}
