//! Headless host adapter (offscreen target, resize, capture/stream cadence).

use std::path::PathBuf;

use bevy::prelude::*;

use crate::viewport::api::ViewportBridgeSet;
use crate::viewport::app::cadence;
use crate::viewport::app::composition;
use crate::viewport::app::headless::HeadlessRenderPlugin;
use crate::viewport::app::launch_options::LaunchOptions;
use crate::viewport::input::{HostUiInputCapture, ViewportNavigationInput};
use crate::viewport::rendering::offscreen::resize::apply_stream_configuration;
use crate::viewport::rendering::sampling::configure_dlss;

/// Configures and executes the headless offscreen `usdview` host without any
/// native windowing (`WinitPlugin` disabled).
pub(crate) fn run(
    launch_options: &LaunchOptions,
    asset_path: &str,
    asset_root: PathBuf,
    is_s8: bool,
) {
    let mut app = App::new();
    configure_dlss(&mut app);
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::asset::AssetPlugin {
                file_path: asset_root.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .set(bevy::log::LogPlugin {
                custom_layer: crate::viewport::diagnostics::log_capture::loader_log_custom_layer,
                ..Default::default()
            }),
    );
    app.init_resource::<HostUiInputCapture>();

    app.add_plugins(HeadlessRenderPlugin {
        width: launch_options.width,
        height: launch_options.height,
    })
    .add_systems(
        Update,
        apply_stream_configuration.before(ViewportBridgeSet::ApplyCommands),
    )
    .insert_resource(ViewportNavigationInput::with_viewport_size(
        launch_options.width,
        launch_options.height,
    ));

    composition::configure_shared_application(
        &mut app,
        launch_options,
        asset_path,
        asset_root,
        is_s8,
    );

    app.set_runner(cadence::run_headless);
    app.run();
}
