use std::sync::mpsc::SyncSender;

use bevy::prelude::App;

use crate::viewport::diagnostics::animation_debug::{AnimationDebugFault, AnimationDebugPlugin};
use crate::viewport::transport::{FrameCapturePlugin, LaunchOptions, parse_launch_options};

pub(super) fn parse_options() -> LaunchOptions {
    match parse_launch_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("usdview: {error}");
            std::process::exit(2);
        }
    }
}

pub(super) fn configure(
    app: &mut App,
    options: &LaunchOptions,
    sender: SyncSender<viewport_streaming::VideoFrame>,
    metrics: viewport_streaming::FrameTransportMetrics,
) -> Result<(), String> {
    if !options.headless {
        return Ok(());
    }
    let fault = if options.animation_debug {
        AnimationDebugFault::parse(options.animation_debug_fault.as_deref())?
    } else {
        AnimationDebugFault::default()
    };
    app.add_plugins(FrameCapturePlugin {
        sender,
        metrics,
        frame_signature: options.animation_debug,
    });
    if options.animation_debug {
        app.add_plugins(AnimationDebugPlugin {
            output_path: options.animation_debug_output.clone(),
            fault,
        });
    }
    Ok(())
}
