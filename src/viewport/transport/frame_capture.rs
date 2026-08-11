//! GPU Frame Readback system for Bevy headless offscreen rendering.
//!
//! Captures rendered pixel buffers from the offscreen `Image` asset each frame
//! and forwards raw RGBA frames into a channel for the WebRTC video encoding pipeline.

use crate::viewport::app::headless::OffscreenTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use std::sync::mpsc::SyncSender;

/// Channel sink resource for pushing rendered video frames to the WebRTC encoder.
#[derive(Resource)]
pub struct FrameCaptureSink {
    pub sender: SyncSender<FrameData>,
}

/// Raw RGBA video frame extracted from the GPU offscreen render target.
#[derive(Clone, Debug)]
pub struct FrameData {
    pub rgba: Vec<u8>,
}

/// Frame capture plugin that registers the frame extraction system in the render schedule.
pub struct FrameCapturePlugin {
    pub sender: SyncSender<FrameData>,
}

impl Plugin for FrameCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FrameCaptureSink {
            sender: self.sender.clone(),
        })
        .add_systems(Startup, setup_frame_readback);
    }
}

fn setup_frame_readback(mut commands: Commands, target: Res<OffscreenTarget>) {
    commands
        .spawn(Readback::texture(target.image_handle.clone()))
        .observe(|event: On<ReadbackComplete>, sink: Res<FrameCaptureSink>| {
            if event.data.is_empty() {
                return;
            }

            let frame = FrameData {
                rgba: event.data.clone(),
            };

            let _ = sink.sender.try_send(frame);
        });
}
