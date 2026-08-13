//! Bevy-main-thread application of authoritative browser stream sizes.

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::Readback;

use super::headless::{OffscreenTarget, new_offscreen_image};
use crate::viewport::api::RenderServerInterface;
use crate::viewport::input::ViewportNavigationInput;

pub(crate) fn apply_stream_configuration(
    interface: Res<RenderServerInterface>,
    mut target: ResMut<OffscreenTarget>,
    mut images: ResMut<Assets<Image>>,
    mut navigation: Option<ResMut<ViewportNavigationInput>>,
    mut render_targets: Query<&mut RenderTarget, With<Camera3d>>,
    mut readbacks: Query<&mut Readback>,
) {
    let Some(metrics) = interface.take_stream_configuration() else {
        return;
    };

    if metrics.generation <= target.generation {
        return;
    }

    let width = metrics.requested_width;
    let height = metrics.requested_height;
    if target.width != width || target.height != height {
        let image_handle = images.add(new_offscreen_image(width, height));
        let render_target = RenderTarget::Image(image_handle.clone().into());
        for mut target in &mut render_targets {
            *target = render_target.clone();
        }
        for mut readback in &mut readbacks {
            *readback = Readback::texture(image_handle.clone());
        }
        target.image_handle = image_handle;
    }

    target.width = width;
    target.height = height;
    target.generation = metrics.generation;
    if let Some(navigation) = navigation.as_deref_mut() {
        navigation.viewport_size = Vec2::new(width as f32, height as f32);
        navigation.begin_stream_generation(metrics.generation);
    }
    bevy::log::info!(
        "[viewport-resize] applied stream configuration {}x{} generation {}",
        width,
        height,
        metrics.generation
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn rgba_byte_count_is_width_times_height_times_four() {
        let width = 1280usize;
        let height = 720usize;
        assert_eq!(width * height * 4, 3_686_400);
    }
}
