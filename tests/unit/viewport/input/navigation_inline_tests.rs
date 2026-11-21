use super::*;

fn navigation_stage() -> usd_bevy::LiveStage {
    let stage = openusd::usd::Stage::builder()
        .in_memory("navigation.usda")
        .unwrap();
    stage.define_prim("/World").unwrap();
    usd_bevy::LiveStage::new(stage)
}

#[test]
fn new_stream_generation_rejects_old_pointer_mapping() {
    let mut input = ViewportNavigationInput::with_viewport_size(1280, 720);
    input.begin_stream_generation(4);
    input.apply_pointer_motion(PointerMotion {
        sequence: 1,
        x_css_pixels: 40.0,
        y_css_pixels: 30.0,
        dx_css_pixels: 12.0,
        dy_css_pixels: 8.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 1280.0,
        viewport_css_height: 720.0,
        stream_generation: 3,
    });
    assert_eq!(input.pointer_delta, Vec2::ZERO);

    input.apply_pointer_motion(PointerMotion {
        sequence: 2,
        x_css_pixels: 48.0,
        y_css_pixels: 36.0,
        dx_css_pixels: 12.0,
        dy_css_pixels: 8.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 1280.0,
        viewport_css_height: 720.0,
        stream_generation: 4,
    });
    assert_eq!(input.pointer_delta, Vec2::new(12.0, 8.0));

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin)
        .init_resource::<ViewportNavigationInput>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<AccumulatedMouseScroll>()
        .insert_resource(usd_bevy::ProjectionBudget::work_items(1))
        .add_systems(
            Update,
            apply_local_navigation_input.after(usd_bevy::LiveStageSet::Project),
        );
    let mut window = Window {
        focused: true,
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(12.0, 14.0)));
    app.world_mut().spawn((window, PrimaryWindow));
    app.world_mut().insert_non_send(navigation_stage());
    app.update();
    let input = app.world().resource::<ViewportNavigationInput>();
    assert!(!input.focused);
    assert_eq!(input.pointer_position, None);
    assert_eq!(app.world().resource::<usd_bevy::PrimEntities>().len(), 1);

    // Removing the cancelled stage restores navigation without waiting for
    // projection to run against a stage that no longer exists.
    app.world_mut()
        .resource_mut::<usd_bevy::ProgressiveProjectionState>()
        .cancel();
    app.world_mut().remove_non_send::<usd_bevy::LiveStage>();
    app.update();
    assert!(app.world().resource::<usd_bevy::PrimEntities>().is_empty());
    assert_eq!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .pointer_position,
        Some(Vec2::new(12.0, 14.0))
    );
}

#[test]
fn remote_motion_uses_authoritative_viewport_cursor_without_publishing_state() {
    let mut input = ViewportNavigationInput::with_viewport_size(100, 80);
    input.apply_pointer_motion(PointerMotion {
        sequence: 1,
        x_css_pixels: 17.0,
        y_css_pixels: 63.0,
        dx_css_pixels: 10.0,
        dy_css_pixels: -5.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 100.0,
        viewport_css_height: 80.0,
        stream_generation: 1,
    });
    assert_eq!(input.pointer_position, Some(Vec2::new(17.0, 63.0)));

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin)
        .init_resource::<ViewportNavigationInput>()
        .init_resource::<crate::viewport::api::RenderServerInterface>()
        .add_systems(
            Update,
            apply_remote_navigation_input.before(usd_bevy::LiveStageSet::Project),
        );
    let send_motion = |app: &App, sequence| {
        app.world()
            .resource::<crate::viewport::api::RenderServerInterface>()
            .submit_pointer_motion(PointerMotion {
                sequence,
                x_css_pixels: 17.0,
                y_css_pixels: 63.0,
                dx_css_pixels: 10.0,
                dy_css_pixels: -5.0,
                wheel_x: 0.0,
                wheel_y: 0.0,
                viewport_css_width: 100.0,
                viewport_css_height: 80.0,
                stream_generation: 1,
            })
            .unwrap();
    };

    app.world_mut().insert_non_send(navigation_stage());
    app.update();
    app.update();
    send_motion(&app, 1);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .pointer_position,
        Some(Vec2::new(17.0, 63.0))
    );

    app.insert_resource(usd_bevy::ProjectionBudget::work_items(1));
    app.world_mut().insert_non_send(navigation_stage());
    send_motion(&app, 2);
    app.update();
    let input = app.world().resource::<ViewportNavigationInput>();
    assert!(
        !input.focused,
        "a completed old session cannot unlock a replacement stage"
    );
    assert_eq!(input.pointer_position, None);

    app.insert_resource(usd_bevy::ProjectionBudget::unlimited());
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .pointer_position,
        None,
        "motion received while locked is discarded"
    );
    send_motion(&app, 3);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .pointer_position,
        Some(Vec2::new(17.0, 63.0))
    );
}
