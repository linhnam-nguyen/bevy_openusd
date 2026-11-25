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
        .add_message::<WindowEvent>()
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
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
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
    // Native events from two distinct positions must retain their positions
    // and order even when four edges are consumed in one Bevy Update.
    for (position, state) in [
        (Vec2::new(3.0, 4.0), bevy::input::ButtonState::Pressed),
        (Vec2::new(3.0, 4.0), bevy::input::ButtonState::Released),
        (Vec2::new(7.0, 8.0), bevy::input::ButtonState::Pressed),
        (Vec2::new(7.0, 8.0), bevy::input::ButtonState::Released),
    ] {
        app.world_mut()
            .write_message(WindowEvent::CursorMoved(CursorMoved {
                window,
                position,
                delta: None,
            }));
        app.world_mut().write_message(WindowEvent::MouseButtonInput(
            bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            },
        ));
    }
    app.update();
    let (edges, overflow) = app
        .world_mut()
        .resource_mut::<ViewportNavigationInput>()
        .take_primary_edges();
    assert!(!overflow);
    assert_eq!(
        edges.iter().map(|edge| edge.pressed).collect::<Vec<_>>(),
        [true, false, true, false]
    );
    assert_eq!(
        edges.iter().map(|edge| edge.position).collect::<Vec<_>>(),
        [
            Some(Vec2::new(3.0, 4.0)),
            Some(Vec2::new(3.0, 4.0)),
            Some(Vec2::new(7.0, 8.0)),
            Some(Vec2::new(7.0, 8.0)),
        ]
    );
    assert!(edges.iter().all(|edge| !edge.modifiers.control));

    // Keyboard transitions and button events must also retain the *press-time*
    // Ctrl modifier, not the end-of-Update key state (which is unpressed).
    let key = |state| {
        WindowEvent::KeyboardInput(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::ControlLeft,
            logical_key: bevy::input::keyboard::Key::Control,
            state,
            text: None,
            repeat: false,
            window,
        })
    };
    app.world_mut()
        .write_message(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: Vec2::new(20.0, 20.0),
            delta: None,
        }));
    app.world_mut()
        .write_message(key(bevy::input::ButtonState::Pressed));
    for state in [
        bevy::input::ButtonState::Pressed,
        bevy::input::ButtonState::Released,
    ] {
        app.world_mut().write_message(WindowEvent::MouseButtonInput(
            bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            },
        ));
    }
    app.world_mut()
        .write_message(key(bevy::input::ButtonState::Released));
    app.world_mut()
        .write_message(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: Vec2::new(30.0, 30.0),
            delta: None,
        }));
    for state in [
        bevy::input::ButtonState::Pressed,
        bevy::input::ButtonState::Released,
    ] {
        app.world_mut().write_message(WindowEvent::MouseButtonInput(
            bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            },
        ));
    }
    app.update();
    let (edges, overflow) = app
        .world_mut()
        .resource_mut::<ViewportNavigationInput>()
        .take_primary_edges();
    assert!(!overflow);
    assert_eq!(edges.len(), 4);
    assert_eq!(
        edges
            .iter()
            .map(|e| e.modifiers.control)
            .collect::<Vec<_>>(),
        [true, true, false, false]
    );
    assert_eq!(edges[0].position, Some(Vec2::new(20.0, 20.0)));
    assert_eq!(edges[2].position, Some(Vec2::new(30.0, 30.0)));

    // A mouse edge after a CursorLeft has no trustworthy position and must
    // fail closed, clearing the input queue instead of selecting at last A/B.
    let previous_epoch = app
        .world()
        .resource::<ViewportNavigationInput>()
        .primary_epoch();
    app.world_mut()
        .write_message(WindowEvent::CursorLeft(CursorLeft { window }));
    for state in [
        bevy::input::ButtonState::Pressed,
        bevy::input::ButtonState::Released,
    ] {
        app.world_mut().write_message(WindowEvent::MouseButtonInput(
            bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            },
        ));
    }
    app.update();
    let mut input = app.world_mut().resource_mut::<ViewportNavigationInput>();
    let (edges, overflow) = input.take_primary_edges();
    assert!(!overflow);
    assert!(edges.is_empty());
    assert!(input.primary_epoch() > previous_epoch);
    drop(input);

    let cursor = |window, position| {
        WindowEvent::CursorMoved(CursorMoved {
            window,
            position,
            delta: None,
        })
    };
    let button = |window, state| {
        WindowEvent::MouseButtonInput(bevy::input::mouse::MouseButtonInput {
            button: MouseButton::Left,
            state,
            window,
        })
    };
    let focus = |window, focused| {
        WindowEvent::WindowFocused(bevy::window::WindowFocused { window, focused })
    };
    let press = bevy::input::ButtonState::Pressed;
    let release = bevy::input::ButtonState::Released;
    let take = |app: &mut App| {
        let (edges, overflow) = app
            .world_mut()
            .resource_mut::<ViewportNavigationInput>()
            .take_primary_edges();
        assert!(!overflow);
        edges
    };

    // A held edge and Ctrl state must be lost across a real focus boundary.
    app.world_mut()
        .write_message(cursor(window, Vec2::splat(40.0)));
    app.world_mut().write_message(key(press));
    app.world_mut().write_message(button(window, press));
    app.update();
    assert!(take(&mut app)[0].modifiers.control);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.world_mut().write_message(focus(window, false));
    app.update();
    let reset_epoch = app
        .world()
        .resource::<ViewportNavigationInput>()
        .primary_epoch();

    // The final window snapshot is focused, but events before focus gain
    // still belong to the previous unfocused observation.
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    for event in [
        cursor(window, Vec2::splat(50.0)),
        button(window, press),
        button(window, release),
        focus(window, true),
        cursor(window, Vec2::splat(60.0)),
        button(window, press),
        button(window, release),
    ] {
        app.world_mut().write_message(event);
    }
    app.update();
    let edges = take(&mut app);
    assert_eq!(edges.len(), 2);
    assert!(
        edges
            .iter()
            .all(|edge| edge.position == Some(Vec2::splat(60.0)) && !edge.modifiers.control)
    );
    assert_eq!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .primary_epoch(),
        reset_epoch
    );

    // Replacing the PrimaryWindow drops the old cursor and pending intent.
    app.world_mut().despawn(window);
    let replacement = app
        .world_mut()
        .spawn((
            Window {
                focused: true,
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    app.world_mut().write_message(button(replacement, press));
    app.world_mut().write_message(button(replacement, release));
    app.update();
    assert!(take(&mut app).is_empty());
    assert!(
        app.world()
            .resource::<ViewportNavigationInput>()
            .primary_epoch()
            > reset_epoch
    );
    app.world_mut()
        .write_message(cursor(replacement, Vec2::splat(70.0)));
    app.world_mut().write_message(button(replacement, press));
    app.world_mut().write_message(button(replacement, release));
    app.update();
    assert_eq!(take(&mut app).len(), 2);
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
        .init_resource::<crate::api::RenderServerInterface>()
        .add_systems(
            Update,
            apply_remote_navigation_input.before(usd_bevy::LiveStageSet::Project),
        );
    let send_motion = |app: &App, sequence| {
        app.world()
            .resource::<crate::api::RenderServerInterface>()
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
