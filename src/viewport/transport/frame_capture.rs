//! GPU Frame Readback system for Bevy headless offscreen rendering.
//!
//! Captures rendered pixel buffers from the offscreen `Image` asset each frame
//! and forwards raw RGBA frames into a channel for the WebRTC video encoding pipeline.

use std::sync::mpsc::SyncSender;
use bevy::prelude::*;
use crate::viewport::app::headless::OffscreenTarget;

/// Channel sink resource for pushing rendered video frames to the WebRTC encoder.
#[derive(Resource)]
pub struct FrameCaptureSink {
    pub sender: SyncSender<FrameData>,
}

/// Raw RGBA video frame extracted from the GPU offscreen render target.
#[derive(Clone, Debug)]
pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub timestamp_ns: u64,
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
        .add_systems(PostUpdate, capture_offscreen_frame_system);
    }
}

/// System that extracts RGBA pixels from the `OffscreenTarget` image asset.
fn capture_offscreen_frame_system(
    target: Option<Res<OffscreenTarget>>,
    sink: Option<Res<FrameCaptureSink>>,
    images: Res<Assets<Image>>,
    time: Res<Time>,
) {
    let (Some(target), Some(sink)) = (target, sink) else {
        return;
    };

    let Some(image) = images.get(&target.image_handle) else {
        return;
    };

    let Some(data) = &image.data else {
        return;
    };

    if data.is_empty() {
        return;
    }

    let frame = FrameData {
        width: target.width,
        height: target.height,
        rgba: data.clone(),
        timestamp_ns: time.elapsed().as_nanos() as u64,
    };

    // Push frame; if encoder channel is full, drop frame to maintain real-time latency
    let _ = sink.sender.try_send(frame);
}
