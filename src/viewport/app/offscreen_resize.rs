//! Bevy-main-thread application of authoritative browser stream sizes.

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::Readback;

use super::cadence::RendererCadence;
use super::headless::{OffscreenTarget, new_offscreen_image};
use crate::viewport::api::RenderServerInterface;
use crate::viewport::input::ViewportNavigationInput;

pub(crate) fn apply_stream_configuration(
    interface: Option<Res<RenderServerInterface>>,
    mut target: ResMut<OffscreenTarget>,
    mut images: ResMut<Assets<Image>>,
    mut navigation: Option<ResMut<ViewportNavigationInput>>,
    mut cadence: Option<ResMut<RendererCadence>>,
    mut render_targets: Query<&mut RenderTarget, With<Camera3d>>,
    mut readbacks: Query<&mut Readback>,
) {
    let Some(interface) = interface else {
        return;
    };
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
    if let Some(cadence) = cadence.as_deref_mut() {
        cadence.request_stream(metrics.preferred_fps, metrics.generation);
    }
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
    use bevy::prelude::*;
    use viewport_protocol::ViewportMetrics;

    use super::*;

    #[test]
    fn rgba_byte_count_is_width_times_height_times_four() {
        let width = 1280usize;
        let height = 720usize;
        assert_eq!(width * height * 4, 3_686_400);
    }

    #[test]
    fn fps_only_stream_update_preserves_offscreen_dimensions() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .insert_resource(RenderServerInterface::default())
            .insert_resource(RendererCadence::new(Some(30)));
        let image_handle = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(new_offscreen_image(1280, 720));
        app.insert_resource(OffscreenTarget {
            image_handle,
            width: 1280,
            height: 720,
            generation: 0,
        })
        .add_systems(Update, apply_stream_configuration);

        app.world()
            .resource::<RenderServerInterface>()
            .shared()
            .submit_stream_configuration(ViewportMetrics {
                requested_width: 1280,
                requested_height: 720,
                preferred_fps: Some(120),
                generation: 1,
                ..Default::default()
            })
            .expect("test stream configuration is valid");

        app.update();

        let target = app.world().resource::<OffscreenTarget>();
        assert_eq!((target.width, target.height), (1280, 720));
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 1);
        assert_eq!(
            app.world().resource::<RendererCadence>().requested_fps(),
            Some(120)
        );
        assert_eq!(
            app.world()
                .resource::<RendererCadence>()
                .effective_renderer_target_fps(),
            Some(30)
        );

        app.world_mut()
            .resource_mut::<RendererCadence>()
            .apply_pending()
            .expect("stream FPS request should be pending");
        assert_eq!(
            app.world()
                .resource::<RendererCadence>()
                .effective_renderer_target_fps(),
            Some(120)
        );
    }
}
