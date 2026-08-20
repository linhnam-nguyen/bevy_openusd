//! GPU Frame Readback system for Bevy headless offscreen rendering.
//!
//! Captures rendered pixel buffers from the offscreen `Image` asset each frame
//! and forwards raw RGBA frames into a channel for the WebRTC video encoding pipeline.

use crate::viewport::app::headless::OffscreenTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::renderer::RenderDevice;
use std::sync::mpsc::SyncSender;
use viewport_streaming::VideoFrame;

/// Channel sink resource for pushing rendered video frames to the WebRTC encoder.
#[derive(Resource)]
pub struct FrameCaptureSink {
    pub sender: SyncSender<VideoFrame>,
}

/// Frame capture plugin that registers the frame extraction system in the render schedule.
pub struct FrameCapturePlugin {
    pub sender: SyncSender<VideoFrame>,
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
        .observe(|event: On<ReadbackComplete>, sink: Res<FrameCaptureSink>, target: Res<OffscreenTarget>, mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>| {
            if event.data.is_empty() {
                return;
            }

            let Some(rgba) = unpack_rgba_readback(&event.data, target.width, target.height) else {
                let row_bytes = (target.width as usize).saturating_mul(4);
                let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
                bevy::log::debug!(
                    "[viewport-frame-capture] dropping {}-byte readback for {}x{} target (expected {} padded bytes)",
                    event.data.len(),
                    target.width,
                    target.height,
                    aligned_row_bytes.saturating_mul(target.height as usize)
                );
                return;
            };

            let frame = VideoFrame {
                rgba,
                width: target.width,
                height: target.height,
                generation: target.generation,
            };

            if let Some(ref mut c) = counters {
                c.captured_frames += 1;
            }

            if sink.sender.try_send(frame).is_err() {
                if let Some(ref mut c) = counters {
                    c.frame_queue_drops += 1;
                }
            }
        });
}

fn unpack_rgba_readback(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }

    let row_bytes = (width as usize).checked_mul(4)?;
    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
    let expected_bytes = aligned_row_bytes.checked_mul(height as usize)?;
    if data.len() != expected_bytes {
        return None;
    }

    if aligned_row_bytes == row_bytes {
        return Some(data.to_vec());
    }

    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in data.chunks_exact(aligned_row_bytes) {
        rgba.extend_from_slice(&row[..row_bytes]);
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::unpack_rgba_readback;
    use bevy::render::renderer::RenderDevice;

    #[test]
    fn strips_gpu_row_padding_from_rgba_readback() {
        let width = 130;
        let height = 2;
        let row_bytes = width * 4;
        let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes as usize);
        let mut data = vec![0u8; aligned_row_bytes * height as usize];
        data[..row_bytes as usize].fill(1);
        data[aligned_row_bytes..aligned_row_bytes + row_bytes as usize].fill(2);

        let rgba = unpack_rgba_readback(&data, width, height).expect("padded readback is valid");

        assert_eq!(rgba.len(), row_bytes as usize * height as usize);
        assert!(rgba[..row_bytes as usize].iter().all(|byte| *byte == 1));
        assert!(rgba[row_bytes as usize..].iter().all(|byte| *byte == 2));
    }

    #[test]
    fn keeps_aligned_rgba_readback_unchanged() {
        let width = 64;
        let height = 2;
        let data = vec![7u8; width as usize * height as usize * 4];

        assert_eq!(unpack_rgba_readback(&data, width, height), Some(data));
    }
}
