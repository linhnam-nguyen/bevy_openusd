use super::*;

#[test]
fn idle_pick_state_requires_no_new_input_or_camera_revision() {
    let input = ViewportNavigationInput::default();
    let mut state = HoveredTarget {
        last_pointer_position: input.pointer_position,
        last_viewport_size: Some(input.viewport_size),
        last_camera_local: Some(Transform::IDENTITY),
        last_camera_global: Some(Transform::IDENTITY),
        last_clip_from_view: Some(Mat4::IDENTITY),
        last_focused: Some(input.focused),
        last_scene_revision: Some(4),
        ..default()
    };

    assert_eq!(state.last_pointer_position, input.pointer_position);
    assert_eq!(state.last_viewport_size, Some(input.viewport_size));
    assert_eq!(state.last_scene_revision, Some(4));

    state.last_pointer_position = Some(Vec2::new(4.0, 2.0));
    assert_ne!(state.last_pointer_position, input.pointer_position);
}

#[test]
fn hover_pick_stats_start_without_raycasts() {
    assert_eq!(HoverPickStats::default().raycasts, 0);
    assert_eq!(HoverPickStats::default().skipped_idle_frames, 0);
}

#[test]
fn remote_css_pointer_maps_to_actual_camera_viewport() {
    let mapped = map_pointer_to_camera_viewport(
        Vec2::new(480.0, 270.0),
        Vec2::new(960.0, 540.0),
        Vec2::new(1920.0, 1080.0),
    );
    assert!(mapped.abs_diff_eq(Vec2::new(960.0, 540.0), 1e-5));
}

#[test]
fn remote_css_pointer_maps_non_two_x_camera_viewport() {
    let mapped = map_pointer_to_camera_viewport(
        Vec2::new(375.0, 225.0),
        Vec2::new(1000.0, 600.0),
        Vec2::new(1500.0, 900.0),
    );
    assert!(mapped.abs_diff_eq(Vec2::new(562.5, 337.5), 1e-5));
    let letterboxed = map_pointer_to_camera_viewport(
        Vec2::new(400.0, 50.0),
        Vec2::new(800.0, 600.0),
        Vec2::new(1600.0, 900.0),
    );
    assert!(letterboxed.abs_diff_eq(Vec2::new(800.0, -50.0), 1e-5));
}

#[test]
fn same_update_remote_taps_retain_reliable_positions() {
    use viewport_protocol::{ButtonState, InputCommand, PointerButtons, PointerMotion};
    let mut nav = crate::viewport::input::ViewportNavigationInput::default();
    let position = |sequence, x, y| PointerMotion {
        sequence,
        x_css_pixels: x,
        y_css_pixels: y,
        dx_css_pixels: 0.0,
        dy_css_pixels: 0.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 320.0,
        viewport_css_height: 240.0,
        stream_generation: 1,
    };
    nav.apply_pointer_motion(position(50, 200.0, 120.0));
    for (index, (pressed, x, y)) in [
        (true, 3., 4.),
        (false, 3., 4.),
        (true, 7., 8.),
        (false, 7., 8.),
    ]
    .into_iter()
    .enumerate()
    {
        let sequence = index as u64 + 1;
        nav.apply_remote_command(InputCommand::PointerMotion(position(sequence, x, y)));
        nav.apply_remote_command(InputCommand::ButtonState(ButtonState {
            sequence,
            buttons: PointerButtons {
                primary: pressed,
                ..default()
            },
            modifiers: default(),
            stream_generation: 1,
        }));
    }
    nav.reset_frame_motion();
    let (edges, overflow) = nav.take_primary_edges();
    assert!(!overflow);
    assert_eq!(
        edges.iter().map(|e| e.pressed).collect::<Vec<_>>(),
        vec![true, false, true, false]
    );
    assert_eq!(edges[0].position, Some(Vec2::new(3.0, 4.0)));
    assert_eq!(edges[3].position, Some(Vec2::new(7.0, 8.0)));
    assert_eq!(nav.pointer_position, Some(Vec2::new(200.0, 120.0)));
    assert!(nav.take_primary_edges().0.is_empty());
    nav.record_primary_edge(true);
    nav.begin_stream_generation(17);
    assert!(nav.take_primary_edges().0.is_empty());
}
#[test]
fn queued_click_rejects_changed_view_and_returned_camera_pose() {
    let mut world = World::new();
    let camera = world.spawn_empty().id();
    let base = PickViewStamp {
        camera,
        world_from_view: Mat4::IDENTITY,
        camera_local: Transform::IDENTITY,
        clip_from_view: Mat4::IDENTITY,
        pending_clip_from_view: None,
        logical_viewport: Vec2::new(800.0, 600.0),
        physical_viewport: Some((UVec2::ZERO, UVec2::new(800, 600))),
        gpu_representation: Some(4),
        section_box_revision: Some(7),
        section_clip_revision: Some(7),
    };
    let click = QueuedPointClick {
        sequence: 1,
        pointer: Vec2::new(200.0, 100.0),
        viewport_size: Vec2::new(800.0, 600.0),
        generation: 10,
        scene_revision: 20,
        view_stamp: base,
        view_epoch: 9,
        additive: false,
    };
    assert!(current_click_view(&click, Some(base), 9, 10, 20));
    assert!(!current_click_view(&click, Some(base), 10, 10, 20));
    // Epoch change rejects V0→V1→V0, not just unequal last matrices.
    assert!(!current_click_view(&click, None, 9, 10, 20));
    assert!(!current_click_view(&click, Some(base), 9, 11, 20));
    assert!(!current_click_view(&click, Some(base), 9, 10, 21));
    for changed in [
        PickViewStamp {
            world_from_view: Mat4::from_translation(Vec3::X),
            ..base
        },
        PickViewStamp {
            camera_local: Transform::from_translation(Vec3::X),
            ..base
        },
        PickViewStamp {
            pending_clip_from_view: Some(Mat4::from_scale(Vec3::splat(2.0))),
            ..base
        },
        PickViewStamp {
            clip_from_view: Mat4::from_scale(Vec3::new(1.2, 1.0, 1.0)),
            ..base
        },
        PickViewStamp {
            logical_viewport: Vec2::new(1024.0, 600.0),
            ..base
        },
        PickViewStamp {
            physical_viewport: Some((UVec2::new(20, 0), UVec2::new(800, 600))),
            ..base
        },
        PickViewStamp {
            gpu_representation: Some(5),
            ..base
        },
        PickViewStamp {
            section_box_revision: Some(8),
            ..base
        },
        PickViewStamp {
            section_clip_revision: Some(8),
            ..base
        },
    ] {
        assert!(!current_click_view(&click, Some(changed), 9, 10, 20));
    }
    let other_camera = world.spawn_empty().id();
    assert!(!current_click_view(
        &click,
        Some(PickViewStamp {
            camera: other_camera,
            ..base
        }),
        9,
        10,
        20,
    ));

    // Three consecutive renderer observations: press at V0; a frame with no
    // button edges and V1; then release. The actual PointClick state must not
    // retain the V0 press after the no-edge frame.
    let primary_down = PrimaryButtonEdge {
        pressed: true,
        position: Some(Vec2::new(200.0, 100.0)),
        modifiers: default(),
        viewport_size: Vec2::new(800.0, 600.0),
        generation: 10,
    };
    let stamped = StampedPress {
        edge: primary_down,
        view_stamp: base,
        view_epoch: 9,
        scene_revision: 20,
    };
    let mut point_click = PointClick::default();
    point_click.press = Some(stamped);
    // Unchanged view (no edges) must keep the press for ordinary and box use.
    point_click.discard_stale_press(base, 9, 10, 20, primary_down.viewport_size);
    assert!(point_click.press.is_some());
    // Intervening observed camera change, with zero edges: cancel immediately.
    point_click.discard_stale_press(base, 10, 10, 20, primary_down.viewport_size);
    assert!(point_click.press.is_none());
    // Once lost, the previous press cannot be resurrected by V1 -> V0.
    point_click.discard_stale_press(base, 11, 10, 20, primary_down.viewport_size);
    assert!(point_click.press.is_none());

    // Independently invalidate scene, stream, viewport and camera transitions.
    for (view, epoch, generation, revision, viewport) in [
        (base, 9, 11, 20, primary_down.viewport_size),
        (base, 9, 10, 21, primary_down.viewport_size),
        (base, 9, 10, 20, Vec2::new(1024.0, 600.0)),
        (
            PickViewStamp {
                camera: other_camera,
                ..base
            },
            9,
            10,
            20,
            primary_down.viewport_size,
        ),
    ] {
        let mut state = PointClick::default();
        state.press = Some(stamped);
        state.discard_stale_press(view, epoch, generation, revision, viewport);
        assert!(state.press.is_none());
    }
    // New, correctly stamped presses can still be accepted on a stable view.
    let mut stable = PointClick::default();
    stable.press = Some(stamped);
    for _ in 0..3 {
        stable.discard_stale_press(base, 9, 10, 20, primary_down.viewport_size);
        assert!(stable.press.is_some());
    }
}

#[test]
fn queued_click_remains_valid_only_in_unchanged_view() {
    let camera = World::new().spawn_empty().id();
    let view = PickViewStamp {
        camera,
        world_from_view: Mat4::IDENTITY,
        camera_local: Transform::IDENTITY,
        clip_from_view: Mat4::IDENTITY,
        pending_clip_from_view: None,
        logical_viewport: Vec2::new(640.0, 480.0),
        physical_viewport: None,
        gpu_representation: None,
        section_box_revision: None,
        section_clip_revision: None,
    };
    let click = QueuedPointClick {
        sequence: 42,
        pointer: Vec2::new(20.0, 30.0),
        viewport_size: Vec2::new(640.0, 480.0),
        generation: 3,
        scene_revision: 5,
        view_stamp: view,
        view_epoch: 2,
        additive: true,
    };
    assert!(current_click_view(&click, Some(view), 2, 3, 5));
}

#[test]
fn front_clipped_rear_surface_is_reached_on_same_mesh_ray() {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-1., -1., 1.],
            [1., -1., 1.],
            [0., 1., 1.],
            [-1., -1., 0.],
            [1., -1., 0.],
            [0., 1., 0.],
        ],
    );
    let ray = Ray3d::new(Vec3::new(0., 0., 2.), Dir3::NEG_Z);
    for indexed in [false, true] {
        if indexed {
            mesh.insert_indices(Indices::U16((0..6).collect()));
        }
        let nearest = |keeps: fn(Vec3) -> bool| {
            nearest_section_clip_triangle(
                ray,
                &mesh,
                &GlobalTransform::IDENTITY,
                Some(Face::Back),
                keeps,
            )
            .unwrap()
        };
        assert_eq!(nearest(|_| true), Some(1.));
        assert_eq!(nearest(|point| (-0.5..=0.5).contains(&point.z)), Some(2.));
        assert_eq!(nearest(|_| false), None);
        let mirrored = GlobalTransform::from(Transform::from_scale(Vec3::new(-1., 1., 1.)));
        assert_eq!(
            nearest_section_clip_triangle(ray, &mesh, &mirrored, Some(Face::Front), |p| p.z <= 0.5),
            Ok(Some(2.))
        );
    }
}

#[test]
fn ray_parallel_to_box_outside_has_no_intersection() {
    assert!(!ray_overlaps_aabb(
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::Y,
        Vec3::splat(-0.5),
        Vec3::splat(0.5),
    ));
}
