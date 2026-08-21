//! GPU Frame Readback system for Bevy headless offscreen rendering.
//!
//! Captures rendered pixel buffers from the offscreen `Image` asset each frame
//! and forwards raw RGBA frames into a channel for the WebRTC video encoding pipeline.

use crate::viewport::app::headless::OffscreenTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::renderer::RenderDevice;
use std::sync::{Arc, mpsc::SyncSender};
use viewport_streaming::{FrameTransportMetrics, VideoFrame};

/// Channel sink resource for pushing rendered video frames to the WebRTC encoder.
#[derive(Resource)]
pub struct FrameCaptureSink {
    pub sender: SyncSender<VideoFrame>,
}

#[derive(Resource, Clone)]
pub(crate) struct FrameTransportResource(pub FrameTransportMetrics);

/// Frame capture plugin that registers the frame extraction system in the render schedule.
pub struct FrameCapturePlugin {
    pub sender: SyncSender<VideoFrame>,
    pub metrics: FrameTransportMetrics,
}

impl Plugin for FrameCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FrameCaptureSink {
            sender: self.sender.clone(),
        })
        .insert_resource(FrameTransportResource(self.metrics.clone()))
        .add_systems(Startup, setup_frame_readback);
    }
}

fn setup_frame_readback(
    mut commands: Commands,
    target: Res<OffscreenTarget>,
    metrics: Res<FrameTransportResource>,
) {
    let metrics = metrics.0.clone();
    commands
        .spawn(Readback::texture(target.image_handle.clone()))
        .observe(move |mut event: On<ReadbackComplete>, sink: Res<FrameCaptureSink>, target: Res<OffscreenTarget>, mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>| {
            metrics.record_readback_completion();
            if event.data.is_empty() {
                metrics.record_invalid_readback();
                return;
            }

            let readback_bytes = event.data.len();
            let data = std::mem::take(&mut event.event_mut().data);
            let Some((rgba, repacked)) = unpack_rgba_readback(data, target.width, target.height) else {
                let row_bytes = (target.width as usize).saturating_mul(4);
                let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
                metrics.record_invalid_readback();
                bevy::log::debug!(
                    "[viewport-frame-capture] dropping {}-byte readback for {}x{} target (expected {} padded bytes)",
                    readback_bytes,
                    target.width,
                    target.height,
                    aligned_row_bytes.saturating_mul(target.height as usize)
                );
                return;
            };

            let trace = metrics.next_trace();
            metrics.record_captured(readback_bytes, repacked);
            let frame = VideoFrame {
                rgba: Arc::new(rgba),
                width: target.width,
                height: target.height,
                generation: target.generation,
                trace,
            };

            if let Some(ref mut c) = counters {
                c.captured_frames += 1;
            }

            if sink.sender.try_send(frame).is_err() {
                metrics.record_queue_full_drop();
                if let Some(ref mut c) = counters {
                    c.frame_queue_drops += 1;
                }
            } else {
                metrics.record_queued(trace);
            }
        });
}

fn unpack_rgba_readback(mut data: Vec<u8>, width: u32, height: u32) -> Option<(Vec<u8>, bool)> {
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
        return Some((data, false));
    }

    // The GPU allocation already owns all bytes. Compact rows in place so a
    // padded readback does not create a second full-frame allocation. The
    // overlapping copy is safe because `copy_within` handles source/dest
    // overlap and rows are moved from the front towards the back.
    for row_index in 1..height as usize {
        let source_start = row_index * aligned_row_bytes;
        let destination_start = row_index * row_bytes;
        data.copy_within(source_start..source_start + row_bytes, destination_start);
    }
    data.truncate(row_bytes * height as usize);
    Some((data, true))
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

        let (rgba, repacked) =
            unpack_rgba_readback(data, width, height).expect("padded readback is valid");

        assert_eq!(rgba.len(), row_bytes as usize * height as usize);
        assert!(rgba[..row_bytes as usize].iter().all(|byte| *byte == 1));
        assert!(rgba[row_bytes as usize..].iter().all(|byte| *byte == 2));
        assert!(repacked);
    }

    #[test]
    fn compacts_padded_rows_without_changing_the_frame_bytes() {
        let width = 130;
        let height = 3;
        let row_bytes = width * 4;
        let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes as usize);
        let mut data = vec![0u8; aligned_row_bytes * height as usize];
        for row in 0..height as usize {
            let start = row * aligned_row_bytes;
            data[start..start + row_bytes as usize].fill((row + 3) as u8);
        }
        let original_capacity = data.capacity();

        let (rgba, repacked) =
            unpack_rgba_readback(data, width, height).expect("padded readback is valid");

        assert!(repacked);
        assert_eq!(rgba.capacity(), original_capacity);
        for row in 0..height as usize {
            let start = row * row_bytes as usize;
            assert!(
                rgba[start..start + row_bytes as usize]
                    .iter()
                    .all(|byte| *byte == (row + 3) as u8)
            );
        }
    }

    #[test]
    fn keeps_aligned_rgba_readback_unchanged() {
        let width = 64;
        let height = 2;
        let data = vec![7u8; width as usize * height as usize * 4];

        assert_eq!(
            unpack_rgba_readback(data.clone(), width, height),
            Some((data, false))
        );
    }
}
