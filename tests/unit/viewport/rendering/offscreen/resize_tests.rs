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
