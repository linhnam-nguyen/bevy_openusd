use bevy::camera::{PerspectiveProjection, Projection, RenderTargetInfo};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use usd_bevy::{Projection as UsdProjection, UsdCamera, UsdPrimRef};

use super::*;
use crate::viewport::navigation::input::{HostUiInputCapture, ViewportNavigationInput};
use crate::viewport::rendering::scene::visualization::SceneExtent;

#[derive(Resource, Default, Clone, Copy)]
struct PendingTestInput {
    wheel_delta: Vec2,
    pointer_delta: Vec2,
    viewport_size: Vec2,
    focused: bool,
    buttons: viewport_protocol::PointerButtons,
    active: bool,
}

fn inject_test_navigation_input(
    mut pending: ResMut<PendingTestInput>,
    mut input: ResMut<ViewportNavigationInput>,
) {
    if !pending.active {
        return;
    }
    input.focused = pending.focused;
    if pending.viewport_size.x > 0.0 && pending.viewport_size.y > 0.0 {
        input.viewport_size = pending.viewport_size;
    }
    input.buttons = pending.buttons;
    input.pointer_delta = pending.pointer_delta;
    input.wheel_delta = pending.wheel_delta;
    pending.wheel_delta = Vec2::ZERO;
    pending.pointer_delta = Vec2::ZERO;
}

fn build_navigation_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ArcballCameraPlugin))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<AccumulatedMouseScroll>()
        .init_resource::<SceneExtent>()
        .init_resource::<FlyTo>()
        .init_resource::<PendingTestInput>()
        .add_systems(
            Update,
            (
                inject_test_navigation_input
                    .after(ArcballCameraSet::PrepareInput)
                    .before(ArcballCameraSet::ApplyInput),
                fit_camera_once.before(sync_adaptive_camera_clipping),
                apply_fly_to.before(sync_adaptive_camera_clipping),
                follow_mounted_camera,
            ),
        );
    app
}

fn spawn_test_camera(
    app: &mut App,
    arcball: ArcballCamera,
    viewport_width: u32,
    viewport_height: u32,
) -> Entity {
    let mut transform = Transform::default();
    apply_rig(&arcball, &mut transform);

    let projection = Projection::Perspective(PerspectiveProjection {
        aspect_ratio: viewport_width as f32 / viewport_height as f32,
        ..default()
    });
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(viewport_width, viewport_height),
        scale_factor: 1.0,
    });
    camera.computed.clip_from_view = projection.get_clip_from_view();

    let pan_orbit = default_pan_orbit_camera(arcball.distance);
    app.world_mut()
        .spawn((
            Camera3d::default(),
            camera,
            projection,
            transform,
            pan_orbit,
            arcball,
        ))
        .id()
}

fn refresh_computed_projection(app: &mut App, cam_entity: Entity) {
    let clip = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("test camera must have a Projection")
        .get_clip_from_view();
    let mut camera = app
        .world_mut()
        .get_mut::<Camera>(cam_entity)
        .expect("test camera must have a Camera");
    camera.computed.clip_from_view = clip;
}

fn send_wheel(app: &mut App, cam_entity: Entity, wheel_y: f32, viewport: Vec2) {
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.active = true;
        pending.focused = true;
        pending.viewport_size = viewport;
        pending.wheel_delta = Vec2::new(0.0, wheel_y);
    }
    app.update();
    refresh_computed_projection(app, cam_entity);
}

#[test]
fn sub_millimetre_object_frames_and_zooms_without_lockup() {
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(&mut app, ArcballCamera::default(), 1280, 720);

    // Spawn a 0.1 mm cube prim centered at origin.
    app.world_mut().spawn((
        UsdPrimRef {
            path: "/MicroPart".to_owned(),
        },
        Transform::default(),
    ));
    *app.world_mut().resource_mut::<SceneExtent>() = SceneExtent {
        min: Vec3::splat(-5.0e-5),
        max: Vec3::splat(5.0e-5),
        count: 1,
        geometry_min: Vec3::splat(-5.0e-5),
        geometry_max: Vec3::splat(5.0e-5),
        geometry_count: 1,
    };

    // Two ticks allow `fit_camera_once` to stabilize and frame the 0.1 mm part.
    app.update();
    app.update();

    let framed = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        framed < 1.0e-3 && framed > 1.0e-5,
        "0.1 mm object must frame close to its real extent, got distance {framed}"
    );

    let Projection::Perspective(proj) = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("projection exists")
    else {
        panic!("expected perspective projection");
    };
    assert!(proj.near.is_finite() && proj.near > 0.0 && proj.near < framed);
    assert!(proj.far.is_finite() && proj.far > framed);

    // Zoom in and out repeatedly at sub-millimetre scale.
    send_wheel(&mut app, cam_entity, 50.0, Vec2::new(1280.0, 720.0));
    let closer = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        closer < framed,
        "scroll up must move closer at 0.1 mm scale"
    );

    send_wheel(&mut app, cam_entity, -50.0, Vec2::new(1280.0, 720.0));
    let restored = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        (restored - framed).abs() / framed < 1.0e-4,
        "reversing scroll must restore sub-millimetre distance: expected {framed}, got {restored}"
    );
}

#[test]
fn millimetre_bolt_in_ten_kilometre_scene_keeps_local_near_and_macro_far() {
    let mut app = build_navigation_test_app();
    // 10 km scene bounds [-5000, +5000].
    *app.world_mut().resource_mut::<SceneExtent>() = SceneExtent {
        min: Vec3::splat(-5_000.0),
        max: Vec3::splat(5_000.0),
        count: 42,
        geometry_min: Vec3::splat(-5_000.0),
        geometry_max: Vec3::splat(5_000.0),
        geometry_count: 42,
    };

    // Inspect a 1 mm bolt located 4 km from the global scene center.
    let bolt_focus = Vec3::new(4_000.0, 12.0, -300.0);
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: bolt_focus,
            distance: 0.002,
            zoom_target: 0.002,
            ..Default::default()
        },
        1280,
        720,
    );

    app.update();
    refresh_computed_projection(&mut app, cam_entity);

    let Projection::Perspective(proj) = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("projection exists")
    else {
        panic!("expected perspective projection");
    };
    assert!(
        proj.near < 0.0005,
        "local near plane must not be floored by 10 km global scene scale: near={}",
        proj.near
    );
    assert!(
        proj.far > 10_000.0,
        "far plane must still enclose the 10 km scene: far={}",
        proj.far
    );

    // Zooming in and out on the 1 mm bolt must remain responsive without lockup.
    send_wheel(&mut app, cam_entity, 40.0, Vec2::new(1280.0, 720.0));
    let zoomed_in = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(zoomed_in < 0.002, "bolt inspection must zoom closer");

    send_wheel(&mut app, cam_entity, -40.0, Vec2::new(1280.0, 720.0));
    let zoomed_back = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        (zoomed_back - 0.002).abs() / 0.002 < 1.0e-4,
        "bolt inspection must zoom back out cleanly"
    );
}

#[test]
fn distant_macro_building_zooms_monotonically_with_ordered_clipping() {
    let mut app = build_navigation_test_app();
    *app.world_mut().resource_mut::<SceneExtent>() = SceneExtent {
        min: Vec3::new(-250_000.0, 0.0, -250_000.0),
        max: Vec3::new(250_000.0, 50_000.0, 250_000.0),
        count: 10,
        geometry_min: Vec3::new(-250_000.0, 0.0, -250_000.0),
        geometry_max: Vec3::new(250_000.0, 50_000.0, 250_000.0),
        geometry_count: 10,
    };
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: Vec3::new(100_000.0, 200.0, -100_000.0),
            distance: 500_000.0,
            zoom_target: 500_000.0,
            ..Default::default()
        },
        1280,
        720,
    );

    app.update();
    let mut prev = 500_000.0_f32;
    for _ in 0..5 {
        send_wheel(&mut app, cam_entity, -60.0, Vec2::new(1280.0, 720.0));
        let next = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        assert!(
            next.is_finite() && next > prev,
            "macro zoom out must be monotonic"
        );
        let Projection::Perspective(proj) = app
            .world()
            .get::<Projection>(cam_entity)
            .expect("projection exists")
        else {
            panic!("expected perspective projection");
        };
        assert!(proj.near.is_finite() && proj.near > 0.0);
        assert!(proj.far.is_finite() && proj.far > proj.near);
        prev = next;
    }
}

#[test]
fn repeated_large_wheel_reversals_stay_monotonic_and_never_collapse_target() {
    let mut app = build_navigation_test_app();
    let initial_focus = Vec3::new(3.0, -2.0, 5.0);
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: initial_focus,
            distance: 10.0,
            zoom_target: 10.0,
            ..Default::default()
        },
        1280,
        720,
    );

    app.update();
    for _ in 0..25 {
        let before = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        // Simulate a large single-frame browser wheel burst (+120 px).
        send_wheel(&mut app, cam_entity, 120.0, Vec2::new(1280.0, 720.0));
        let closer = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        assert!(
            closer.is_finite() && closer > 0.0 && closer < before,
            "large scroll-in burst must shrink distance monotonically without overshooting"
        );

        send_wheel(&mut app, cam_entity, -120.0, Vec2::new(1280.0, 720.0));
        let restored = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        assert!(
            (restored - before).abs() / before < 1.0e-4,
            "reversing large burst must restore distance: before={before}, restored={restored}"
        );
    }

    let cam = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists");
    assert!((cam.focus - initial_focus).length() < 1.0e-5);
    let pan_orbit = app
        .world()
        .get::<PanOrbitCamera>(cam_entity)
        .expect("pan_orbit exists");
    assert!(pan_orbit.last_anchor_depth.is_finite() && pan_orbit.last_anchor_depth < 0.0);
}

#[test]
fn corrupted_and_clipped_pan_orbit_anchor_recovers_in_both_directions() {
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 0.01,
            zoom_target: 0.01,
            ..Default::default()
        },
        1280,
        720,
    );

    for bad_anchor in [f64::NAN, 5.0, 0.0, -1.0e-15] {
        // Corrupt anchor depth and push near plane past the anchor to reproduce
        // the legacy invalid-projection zero-motion lockup trap.
        {
            let mut pan_orbit = app
                .world_mut()
                .get_mut::<PanOrbitCamera>(cam_entity)
                .expect("pan_orbit exists");
            pan_orbit.last_anchor_depth = bad_anchor;
        }
        {
            let mut proj = app
                .world_mut()
                .get_mut::<Projection>(cam_entity)
                .expect("projection exists");
            if let Projection::Perspective(p) = proj.as_mut() {
                p.near = 10.0;
                p.far = 20.0;
            }
        }
        refresh_computed_projection(&mut app, cam_entity);

        let before = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        send_wheel(&mut app, cam_entity, 20.0, Vec2::new(1280.0, 720.0));
        let after_in = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        assert!(
            after_in < before,
            "zoom-in must recover from corrupted anchor {bad_anchor}: before={before}, after={after_in}"
        );

        send_wheel(&mut app, cam_entity, -40.0, Vec2::new(1280.0, 720.0));
        let after_out = app
            .world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance;
        assert!(
            after_out > after_in,
            "zoom-out must recover from corrupted anchor {bad_anchor}: in={after_in}, out={after_out}"
        );
    }
}

#[test]
fn wheel_zoom_ratio_is_independent_of_viewport_resolution_and_scale() {
    let mut low_res = build_navigation_test_app();
    let low_cam = spawn_test_camera(
        &mut low_res,
        ArcballCamera {
            distance: 1.0e-4,
            zoom_target: 1.0e-4,
            ..Default::default()
        },
        640,
        360,
    );
    low_res.update();
    send_wheel(&mut low_res, low_cam, -25.0, Vec2::new(640.0, 360.0));
    let low_ratio = low_res
        .world()
        .get::<ArcballCamera>(low_cam)
        .expect("camera exists")
        .distance
        / 1.0e-4;

    let mut high_res = build_navigation_test_app();
    let high_cam = spawn_test_camera(
        &mut high_res,
        ArcballCamera {
            distance: 1.0e6,
            zoom_target: 1.0e6,
            ..Default::default()
        },
        3840,
        2160,
    );
    high_res.update();
    send_wheel(&mut high_res, high_cam, -25.0, Vec2::new(3840.0, 2160.0));
    let high_ratio = high_res
        .world()
        .get::<ArcballCamera>(high_cam)
        .expect("camera exists")
        .distance
        / 1.0e6;

    assert!(
        (low_ratio - high_ratio).abs() / low_ratio < 1.0e-5,
        "zoom ratio must be scale- and resolution-independent: low={low_ratio}, high={high_ratio}"
    );
}

#[test]
fn pan_and_orbit_work_after_closeup_zoom_and_with_simultaneous_wheel() {
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 1.0e-3,
            zoom_target: 1.0e-3,
            ..Default::default()
        },
        1280,
        720,
    );
    app.update();
    refresh_computed_projection(&mut app, cam_entity);

    // Zoom into sub-millimetre close-up first.
    send_wheel(&mut app, cam_entity, 100.0, Vec2::new(1280.0, 720.0));
    let closeup_dist = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(closeup_dist < 5.0e-4);

    // Middle-button pan + simultaneous wheel scroll.
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.active = true;
        pending.focused = true;
        pending.viewport_size = Vec2::new(1280.0, 720.0);
        pending.buttons.auxiliary = true;
        pending.pointer_delta = Vec2::new(30.0, -20.0);
        pending.wheel_delta = Vec2::new(0.0, -20.0);
    }
    app.update();
    app.update();
    let after_pan = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists");
    assert!(after_pan.focus.is_finite() && after_pan.focus.length() > 0.0);
    assert!(after_pan.distance > closeup_dist);

    // Left + Right orbit after close-up pan.
    let yaw_before = after_pan.yaw;
    let elev_before = after_pan.elevation;
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.buttons.auxiliary = false;
        pending.buttons.primary = true;
        pending.buttons.secondary = true;
        pending.pointer_delta = Vec2::new(50.0, 15.0);
        pending.wheel_delta = Vec2::ZERO;
    }
    app.update();
    app.update();
    let after_orbit = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists");
    assert!(
        (after_orbit.yaw - yaw_before).abs() > 1.0e-3
            || (after_orbit.elevation - elev_before).abs() > 1.0e-3,
        "orbit must remain responsive after close-up zoom and pan"
    );
    assert!(after_orbit.focus.is_finite());
}

#[test]
fn mounted_authored_usd_camera_preserves_explicit_clipping_and_transform() {
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(&mut app, ArcballCamera::default(), 1280, 720);

    let authored_transform = Transform::from_xyz(12.0, 5.0, -8.0).looking_at(Vec3::ZERO, Vec3::Y);
    let authored_projection = Projection::Perspective(PerspectiveProjection {
        fov: 0.6,
        aspect_ratio: 16.0 / 9.0,
        near: 0.25,
        far: 250.0,
        ..default()
    });
    app.world_mut().spawn((
        UsdCamera::default(),
        UsdPrimRef {
            path: "/World/ShotCam".to_owned(),
        },
        authored_transform,
        GlobalTransform::from(authored_transform),
        UsdProjection::from(authored_projection.clone()),
    ));

    *app.world_mut().resource_mut::<CameraMount>() = CameraMount::Mounted {
        prim_path: "/World/ShotCam".to_owned(),
    };
    *app.world_mut().resource_mut::<SceneExtent>() = SceneExtent {
        min: Vec3::splat(-5_000.0),
        max: Vec3::splat(5_000.0),
        count: 10,
        geometry_min: Vec3::splat(-5_000.0),
        geometry_max: Vec3::splat(5_000.0),
        geometry_count: 10,
    };

    // Attempt free-camera scroll and orbit while mounted; authored projection
    // and transform must remain completely untouched.
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.active = true;
        pending.focused = true;
        pending.wheel_delta = Vec2::new(0.0, 100.0);
        pending.buttons.primary = true;
        pending.buttons.secondary = true;
        pending.pointer_delta = Vec2::new(80.0, 40.0);
    }
    app.update();
    app.update();

    let Projection::Perspective(live_proj) = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("projection exists")
    else {
        panic!("expected perspective projection");
    };
    assert!(
        (live_proj.near - 0.25).abs() < 1.0e-6,
        "mounted authored camera near plane must not be overwritten: {}",
        live_proj.near
    );
    assert!(
        (live_proj.far - 250.0).abs() < 1.0e-4,
        "mounted authored camera far plane must not be overwritten: {}",
        live_proj.far
    );
    assert!((live_proj.fov - 0.6).abs() < 1.0e-6);
    let live_tr = app
        .world()
        .get::<Transform>(cam_entity)
        .expect("transform exists");
    assert!((live_tr.translation - authored_transform.translation).length() < 1.0e-5);

    // Switching back to free camera restores local clipping authority and wheel zoom.
    *app.world_mut().resource_mut::<CameraMount>() = CameraMount::Arcball;
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.buttons = Default::default();
        pending.pointer_delta = Vec2::ZERO;
        pending.wheel_delta = Vec2::new(0.0, 50.0);
    }
    app.world_mut()
        .resource_mut::<HostUiInputCapture>()
        .wants_pointer = false;
    let dist_before = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    app.update();
    let dist_after = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(dist_after < dist_before);
    let Projection::Perspective(free_proj) = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("projection exists")
    else {
        panic!("expected perspective projection");
    };
    assert!(
        free_proj.far > 9_000.0,
        "free camera far plane must enclose the 10 km scene bounds (radius ~8660 m), got {}",
        free_proj.far
    );
}

#[test]
fn exponential_dolly_is_invariant_to_frame_partition_inward_and_outward() {
    let viewport = Vec2::new(1280.0, 720.0);

    // Inward large in-range partition: 1e10 with +3000 once vs 2 x +1500.
    let mut single_in = build_navigation_test_app();
    let single_in_cam = spawn_test_camera(
        &mut single_in,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 1.0e10,
            zoom_target: 1.0e10,
            ..Default::default()
        },
        1280,
        720,
    );
    single_in.update();
    send_wheel(&mut single_in, single_in_cam, 3000.0, viewport);

    let mut split_in = build_navigation_test_app();
    let split_in_cam = spawn_test_camera(
        &mut split_in,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 1.0e10,
            zoom_target: 1.0e10,
            ..Default::default()
        },
        1280,
        720,
    );
    split_in.update();
    send_wheel(&mut split_in, split_in_cam, 1500.0, viewport);
    send_wheel(&mut split_in, split_in_cam, 1500.0, viewport);

    let d_single = single_in
        .world()
        .get::<ArcballCamera>(single_in_cam)
        .expect("camera exists")
        .distance;
    let d_split = split_in
        .world()
        .get::<ArcballCamera>(split_in_cam)
        .expect("camera exists")
        .distance;
    let expected = (1.0e10_f64 * (-30.0_f64).exp()) as f32;
    assert!(
        (d_single - expected).abs() / expected < 1.0e-5,
        "single +3000 must reach ~{expected}, got {d_single}"
    );
    assert!(
        (d_single - d_split).abs() / d_single < 1.0e-5,
        "frame partition mismatch: single={d_single}, split={d_split}"
    );
    let tr_single = single_in
        .world()
        .get::<Transform>(single_in_cam)
        .expect("transform exists")
        .translation;
    let tr_split = split_in
        .world()
        .get::<Transform>(split_in_cam)
        .expect("transform exists")
        .translation;
    assert!(
        (tr_single - tr_split).length() / tr_single.length().max(1.0e-6) < 1.0e-5,
        "actual Transform pose must match across partitions: single={tr_single:?}, split={tr_split:?}"
    );

    // Outward partition: 1e-3 with -2000 once vs 2 x -1000.
    let mut single_out = build_navigation_test_app();
    let single_out_cam = spawn_test_camera(
        &mut single_out,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 1.0e-3,
            zoom_target: 1.0e-3,
            ..Default::default()
        },
        1280,
        720,
    );
    single_out.update();
    send_wheel(&mut single_out, single_out_cam, -2000.0, viewport);

    let mut split_out = build_navigation_test_app();
    let split_out_cam = spawn_test_camera(
        &mut split_out,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 1.0e-3,
            zoom_target: 1.0e-3,
            ..Default::default()
        },
        1280,
        720,
    );
    split_out.update();
    send_wheel(&mut split_out, split_out_cam, -1000.0, viewport);
    send_wheel(&mut split_out, split_out_cam, -1000.0, viewport);

    let out_single = single_out
        .world()
        .get::<ArcballCamera>(single_out_cam)
        .expect("camera exists")
        .distance;
    let out_split = split_out
        .world()
        .get::<ArcballCamera>(split_out_cam)
        .expect("camera exists")
        .distance;
    assert!((out_single - out_split).abs() / out_single < 1.0e-5);

    // Ordinary partition: +120 once vs 3 x +40.
    let mut ord_single = build_navigation_test_app();
    let ord_single_cam = spawn_test_camera(&mut ord_single, ArcballCamera::default(), 1280, 720);
    ord_single.update();
    send_wheel(&mut ord_single, ord_single_cam, 120.0, viewport);

    let mut ord_split = build_navigation_test_app();
    let ord_split_cam = spawn_test_camera(&mut ord_split, ArcballCamera::default(), 1280, 720);
    ord_split.update();
    for _ in 0..3 {
        send_wheel(&mut ord_split, ord_split_cam, 40.0, viewport);
    }
    let os = ord_single
        .world()
        .get::<ArcballCamera>(ord_single_cam)
        .expect("camera exists")
        .distance;
    let op = ord_split
        .world()
        .get::<ArcballCamera>(ord_split_cam)
        .expect("camera exists")
        .distance;
    assert!((os - op).abs() / os < 1.0e-5);
}

#[test]
fn exponential_dolly_saturates_huge_finite_deltas_and_reverses_immediately() {
    let viewport = Vec2::new(1280.0, 720.0);
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(&mut app, ArcballCamera::default(), 1280, 720);
    app.update();

    // Huge finite inward delta saturates cleanly at MIN_CAMERA_DISTANCE.
    send_wheel(&mut app, cam_entity, f32::MAX, viewport);
    let at_min = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        (f64::from(at_min) - MIN_CAMERA_DISTANCE).abs() / MIN_CAMERA_DISTANCE < 1.0e-4,
        "huge inward delta must saturate at MIN_CAMERA_DISTANCE, got {at_min}"
    );
    let tr_min = app
        .world()
        .get::<Transform>(cam_entity)
        .expect("transform exists")
        .translation;
    assert!(tr_min.is_finite() && tr_min.length() > 0.0);

    // Opposite scroll immediately reverses away from the lower bound.
    send_wheel(&mut app, cam_entity, -50.0, viewport);
    let after_rev = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        after_rev > at_min * 1.5,
        "reversal from lower endpoint must move away immediately: min={at_min}, after={after_rev}"
    );

    // Huge finite outward delta saturates cleanly at MAX_CAMERA_DISTANCE.
    send_wheel(&mut app, cam_entity, -f32::MAX, viewport);
    let at_max = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(
        (f64::from(at_max) - MAX_CAMERA_DISTANCE).abs() / MAX_CAMERA_DISTANCE < 1.0e-4,
        "huge outward delta must saturate at MAX_CAMERA_DISTANCE, got {at_max}"
    );
    let Projection::Perspective(proj) = app
        .world()
        .get::<Projection>(cam_entity)
        .expect("projection exists")
    else {
        panic!("expected perspective projection");
    };
    assert!(
        proj.near.is_finite() && proj.far.is_finite() && proj.near > 0.0 && proj.far > proj.near
    );

    // Opposite scroll immediately reverses away from the upper bound.
    send_wheel(&mut app, cam_entity, 50.0, viewport);
    let after_max_rev = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    assert!(after_max_rev < at_max * 0.7);

    // Invalid NaN/Inf deltas and invalid speed are no-ops.
    send_wheel(&mut app, cam_entity, f32::NAN, viewport);
    send_wheel(&mut app, cam_entity, f32::INFINITY, viewport);
    assert_eq!(
        app.world()
            .get::<ArcballCamera>(cam_entity)
            .expect("camera exists")
            .distance,
        after_max_rev
    );
    assert_eq!(exponential_dolly_distance(10.0, 5.0, -1.0), 10.0);
    assert_eq!(exponential_dolly_distance(10.0, 5.0, f64::NAN), 10.0);
}

#[test]
fn nonfinite_pan_overflow_skips_pan_without_poisoning_wheel_or_subsequent_gestures() {
    let viewport = Vec2::new(1280.0, 720.0);
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: Vec3::new(1.0, 2.0, 3.0),
            distance: 10.0,
            zoom_target: 10.0,
            pan_sensitivity: 2.0,
            ..Default::default()
        },
        1280,
        720,
    );
    app.update();
    refresh_computed_projection(&mut app, cam_entity);

    // Case 1: Finite f32::MAX component x pan_multiplier (2.0) overflows f32,
    // while simultaneous valid wheel dolly MUST still apply cleanly.
    app.world_mut()
        .resource_mut::<ViewportNavigationInput>()
        .pan_multiplier = 2.0;
    let dist_before = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .distance;
    let tr_before = app
        .world()
        .get::<Transform>(cam_entity)
        .expect("transform exists")
        .translation;
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.active = true;
        pending.focused = true;
        pending.viewport_size = viewport;
        pending.buttons.auxiliary = true;
        pending.pointer_delta = Vec2::new(f32::MAX, 0.0);
        pending.wheel_delta = Vec2::new(0.0, 50.0);
    }
    app.update();
    app.update();

    let cam_after_1 = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists");
    let tr_after_1 = app
        .world()
        .get::<Transform>(cam_entity)
        .expect("transform exists");
    assert!(cam_after_1.focus.is_finite());
    assert!(tr_after_1.translation.is_finite() && tr_after_1.rotation.is_finite());
    assert!(
        cam_after_1.distance < dist_before,
        "valid simultaneous wheel dolly must still apply when pan overflows"
    );
    assert!((tr_after_1.translation - tr_before).length() > 0.1);

    // Case 2: Finite first product (f32::MAX * 0.4 * 2.0 is finite), then
    // pan_sensitivity (2.0) causes final overflow.
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.pointer_delta = Vec2::new(f32::MAX * 0.4, 0.0);
        pending.wheel_delta = Vec2::ZERO;
    }
    app.update();
    app.update();
    assert!(
        app.world()
            .get::<Transform>(cam_entity)
            .expect("transform exists")
            .translation
            .is_finite()
    );

    // Case 3: Raw NaN / Inf pointer delta must not poison camera state.
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.pointer_delta = Vec2::new(f32::NAN, f32::INFINITY);
    }
    app.update();
    app.update();
    assert!(
        app.world()
            .get::<Transform>(cam_entity)
            .expect("transform exists")
            .translation
            .is_finite()
    );

    // Case 4: Next valid pan and orbit gestures recover immediately.
    let focus_before_valid_pan = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .focus;
    app.world_mut()
        .resource_mut::<ViewportNavigationInput>()
        .pan_multiplier = 1.0;
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.buttons.auxiliary = true;
        pending.pointer_delta = Vec2::new(35.0, -25.0);
    }
    app.update();
    app.update();
    let focus_after_valid_pan = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .focus;
    assert!(
        focus_after_valid_pan.is_finite()
            && (focus_after_valid_pan - focus_before_valid_pan).length() > 1.0e-3,
        "valid pan must recover immediately after discarded overflow pan"
    );

    let yaw_before = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .yaw;
    {
        let mut pending = app.world_mut().resource_mut::<PendingTestInput>();
        pending.buttons.auxiliary = false;
        pending.buttons.primary = true;
        pending.buttons.secondary = true;
        pending.pointer_delta = Vec2::new(40.0, 10.0);
    }
    app.update();
    app.update();
    let yaw_after = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .yaw;
    assert!(
        (yaw_after - yaw_before).abs() > 1.0e-3,
        "valid orbit must recover immediately after discarded overflow pan"
    );
}

#[test]
fn deep_narrow_orthographic_fit_survives_repeated_plugin_ticks_and_navigation() {
    use bevy::camera::{CameraProjection, OrthographicProjection, ScalingMode};

    let mut app = build_navigation_test_app();
    let mut ortho = OrthographicProjection {
        scaling_mode: ScalingMode::WindowSize,
        ..OrthographicProjection::default_3d()
    };
    ortho.update(1280.0, 720.0);

    let arcball = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        distance: 5.0,
        zoom_target: 5.0,
        ..Default::default()
    };
    let mut transform = Transform::default();
    apply_rig(&arcball, &mut transform);

    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    let projection = Projection::Orthographic(ortho);
    camera.computed.clip_from_view = projection.get_clip_from_view();

    let cam_entity = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            camera,
            projection,
            transform,
            default_pan_orbit_camera(arcball.distance),
            arcball,
        ))
        .id();

    // Deep narrow object: width/height 2 m, depth 400 m.
    let min = Vec3::new(-1.0, -1.0, -200.0);
    let max = Vec3::new(1.0, 1.0, 200.0);
    app.world_mut().spawn((
        UsdPrimRef {
            path: "/DeepCorridor".to_owned(),
        },
        Transform::default(),
    ));
    *app.world_mut().resource_mut::<SceneExtent>() = SceneExtent {
        min,
        max,
        count: 1,
        geometry_min: min,
        geometry_max: max,
        geometry_count: 1,
    };

    // Stabilize initial fit and verify containment across 1, 2, and 10 plugin ticks.
    for tick in 1..=12 {
        app.update();
        refresh_computed_projection(&mut app, cam_entity);
        if matches!(tick, 3 | 4 | 12) {
            let tr = *app
                .world()
                .get::<Transform>(cam_entity)
                .expect("transform exists");
            let Projection::Orthographic(live_ortho) = app
                .world()
                .get::<Projection>(cam_entity)
                .expect("projection exists")
            else {
                panic!("expected orthographic projection");
            };
            let view_from_world = tr.compute_affine().inverse();
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                );
                let view = view_from_world.transform_point3(corner);
                let depth = -view.z;
                assert!(
                    depth > live_ortho.near && depth < live_ortho.far,
                    "tick {tick}: ortho corner {corner:?} depth {depth} outside ({}, {})",
                    live_ortho.near,
                    live_ortho.far
                );
                assert!(
                    view.x >= live_ortho.area.min.x - 1.0e-3
                        && view.x <= live_ortho.area.max.x + 1.0e-3
                        && view.y >= live_ortho.area.min.y - 1.0e-3
                        && view.y <= live_ortho.area.max.y + 1.0e-3,
                    "tick {tick}: ortho corner {corner:?} view={view:?} outside area {:?}",
                    live_ortho.area
                );
            }
        }
    }

    // Subsequent wheel zoom in/out reversal remains coherent in orthographic mode.
    let scale_before = match app.world().get::<Projection>(cam_entity).expect("proj") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    send_wheel(&mut app, cam_entity, 40.0, Vec2::new(1280.0, 720.0));
    let scale_in = match app.world().get::<Projection>(cam_entity).expect("proj") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    assert!(scale_in < scale_before);
    send_wheel(&mut app, cam_entity, -40.0, Vec2::new(1280.0, 720.0));
    let scale_back = match app.world().get::<Projection>(cam_entity).expect("proj") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    assert!((scale_back - scale_before).abs() / scale_before < 1.0e-4);
}

#[test]
fn active_fly_to_is_cancelled_on_authored_mount_and_never_resumes_on_unmount() {
    use bevy::camera::OrthographicProjection;

    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            focus: Vec3::ZERO,
            distance: 10.0,
            zoom_target: 10.0,
            ..Default::default()
        },
        1280,
        720,
    );

    let authored_tr = Transform::from_xyz(30.0, 12.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y);
    let authored_proj = Projection::Perspective(PerspectiveProjection {
        fov: 0.55,
        aspect_ratio: 16.0 / 9.0,
        near: 0.4,
        far: 400.0,
        ..default()
    });
    app.world_mut().spawn((
        UsdCamera::default(),
        UsdPrimRef {
            path: "/World/AuthoredCam".to_owned(),
        },
        authored_tr,
        GlobalTransform::from(authored_tr),
        UsdProjection::from(authored_proj.clone()),
    ));

    // Start an active FlyTo tween carrying both perspective pose and an ortho scale target.
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: Vec3::ZERO,
        start_distance: 10.0,
        target_focus: Vec3::new(50.0, 20.0, 10.0),
        target_distance: 2.0,
        start_ortho_scale: Some(1.0),
        target_ortho_scale: Some(0.05),
        duration: 1.0,
        remaining: 1.0,
        ..Default::default()
    };

    // Mount authored USD camera while tween is active.
    *app.world_mut().resource_mut::<CameraMount>() = CameraMount::Mounted {
        prim_path: "/World/AuthoredCam".to_owned(),
    };
    for _ in 0..5 {
        app.update();
        let tr = app
            .world()
            .get::<Transform>(cam_entity)
            .expect("transform exists");
        assert!(
            (tr.translation - authored_tr.translation).length() < 1.0e-5,
            "active FlyTo must not overwrite mounted authored camera transform"
        );
        let Projection::Perspective(proj) = app
            .world()
            .get::<Projection>(cam_entity)
            .expect("projection exists")
        else {
            panic!("expected authored perspective projection");
        };
        assert!((proj.near - 0.4).abs() < 1.0e-6);
        assert!((proj.far - 400.0).abs() < 1.0e-4);
    }

    // Active tween must have been cancelled completely (including ortho fields).
    let fly = app.world().resource::<FlyTo>();
    assert_eq!(fly.remaining, 0.0);
    assert_eq!(fly.target_ortho_scale, None);

    // Unmounting back to Arcball must NOT resume the cancelled tween.
    *app.world_mut().resource_mut::<CameraMount>() = CameraMount::Arcball;
    let focus_at_unmount = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .focus;
    app.update();
    app.update();
    let focus_after_unmount = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists")
        .focus;
    assert_eq!(
        focus_after_unmount, focus_at_unmount,
        "cancelled tween must never resume after unmounting authored camera"
    );

    // Switching projection from Orthographic to Perspective mid-tween cancels cleanly.
    *app.world_mut()
        .get_mut::<Projection>(cam_entity)
        .expect("proj") = Projection::Orthographic(OrthographicProjection::default_3d());
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: Vec3::ZERO,
        start_distance: 10.0,
        target_focus: Vec3::new(5.0, 0.0, 0.0),
        target_distance: 4.0,
        start_ortho_scale: Some(1.0),
        target_ortho_scale: Some(0.25),
        duration: 1.0,
        remaining: 1.0,
        ..Default::default()
    };
    *app.world_mut()
        .get_mut::<Projection>(cam_entity)
        .expect("proj") = Projection::Perspective(PerspectiveProjection::default());
    app.update();
    assert_eq!(app.world().resource::<FlyTo>().remaining, 0.0);
    assert_eq!(app.world().resource::<FlyTo>().target_ortho_scale, None);
}

#[test]
fn orthographic_wheel_zooms_below_distance_floor_is_partition_invariant_and_decoupled_from_distance()
 {
    use bevy::camera::{CameraProjection, OrthographicProjection, ScalingMode};

    let viewport = Vec2::new(3840.0, 2160.0);
    let spawn_ortho = |app: &mut App, distance: f32, scale: f32| -> Entity {
        let mut ortho = OrthographicProjection {
            scaling_mode: ScalingMode::WindowSize,
            scale,
            ..OrthographicProjection::default_3d()
        };
        ortho.update(viewport.x, viewport.y);
        let arcball = ArcballCamera {
            yaw: 0.0,
            elevation: 0.0,
            distance,
            zoom_target: f64::from(distance),
            ..Default::default()
        };
        let mut transform = Transform::default();
        apply_rig(&arcball, &mut transform);
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(3840, 2160),
            scale_factor: 1.0,
        });
        let projection = Projection::Orthographic(ortho);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        app.world_mut()
            .spawn((
                Camera3d::default(),
                camera,
                projection,
                transform,
                default_pan_orbit_camera(distance),
                arcball,
            ))
            .id()
    };

    // 1. Inward ortho wheel below 1e-7 shrinks area without moving camera depth.
    let mut app = build_navigation_test_app();
    let initial_scale = 5.0926e-8_f32;
    let cam_entity = spawn_ortho(&mut app, 400.0, initial_scale);
    app.update();
    let tr_before = *app.world().get::<Transform>(cam_entity).expect("tr");
    let dist_before = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("cam")
        .distance;
    send_wheel(&mut app, cam_entity, 50.0, viewport);
    let Projection::Orthographic(after_in) = app.world().get::<Projection>(cam_entity).expect("p")
    else {
        panic!("expected ortho");
    };
    assert!(
        after_in.scale < initial_scale * 0.7,
        "inward ortho wheel below 1e-7 must shrink scale: before={initial_scale}, after={}",
        after_in.scale
    );
    assert!(after_in.area.height() < initial_scale * 2160.0 * 0.7);
    assert_eq!(
        app.world()
            .get::<ArcballCamera>(cam_entity)
            .expect("cam")
            .distance,
        dist_before,
        "ortho magnification must not translate camera in depth"
    );
    assert_eq!(
        app.world()
            .get::<Transform>(cam_entity)
            .expect("tr")
            .translation,
        tr_before.translation
    );

    // 2. Partition invariance: +120 once vs 3 x +40.
    let mut single = build_navigation_test_app();
    let single_cam = spawn_ortho(&mut single, 10.0, 1.0e-4);
    single.update();
    send_wheel(&mut single, single_cam, 120.0, viewport);

    let mut split = build_navigation_test_app();
    let split_cam = spawn_ortho(&mut split, 10.0, 1.0e-4);
    split.update();
    for _ in 0..3 {
        send_wheel(&mut split, split_cam, 40.0, viewport);
    }
    let s_scale = match single.world().get::<Projection>(single_cam).expect("p") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    let p_scale = match split.world().get::<Projection>(split_cam).expect("p") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    assert!(
        (s_scale - p_scale).abs() / s_scale < 1.0e-5,
        "ortho wheel partition mismatch: single={s_scale}, split={p_scale}"
    );

    // 3. Decoupled from camera distance endpoints: wheel still zooms when
    // distance sits at MIN_CAMERA_DISTANCE or MAX_CAMERA_DISTANCE.
    for endpoint_dist in [MIN_CAMERA_DISTANCE as f32, MAX_CAMERA_DISTANCE as f32] {
        let mut ep = build_navigation_test_app();
        let ep_cam = spawn_ortho(&mut ep, endpoint_dist, 1.0e-3);
        ep.update();
        send_wheel(&mut ep, ep_cam, 50.0, viewport);
        let scaled_in = match ep.world().get::<Projection>(ep_cam).expect("p") {
            Projection::Orthographic(o) => o.scale,
            _ => panic!("expected ortho"),
        };
        assert!(
            scaled_in < 1.0e-3 * 0.7,
            "ortho zoom must not stall at distance endpoint {endpoint_dist}"
        );
        send_wheel(&mut ep, ep_cam, -50.0, viewport);
        let scaled_back = match ep.world().get::<Projection>(ep_cam).expect("p") {
            Projection::Orthographic(o) => o.scale,
            _ => panic!("expected ortho"),
        };
        assert!((scaled_back - 1.0e-3).abs() / 1.0e-3 < 1.0e-4);
    }

    // 4. True scale endpoint saturation + immediate reversal.
    send_wheel(&mut app, cam_entity, f32::MAX, viewport);
    let at_min_scale = match app.world().get::<Projection>(cam_entity).expect("p") {
        Projection::Orthographic(o) => {
            assert!(
                o.get_clip_from_view()
                    .to_cols_array()
                    .iter()
                    .all(|v| v.is_finite())
            );
            o.scale
        }
        _ => panic!("expected ortho"),
    };
    send_wheel(&mut app, cam_entity, -50.0, viewport);
    let rev_scale = match app.world().get::<Projection>(cam_entity).expect("p") {
        Projection::Orthographic(o) => o.scale,
        _ => panic!("expected ortho"),
    };
    assert!(rev_scale > at_min_scale * 1.5);
}

#[test]
fn active_focus_transition_adapts_to_mid_tween_resize_fov_and_projection_switch() {
    use bevy::camera::{CameraProjection, OrthographicProjection, ScalingMode};

    let min = Vec3::new(-1.0, -0.05, -0.05);
    let max = Vec3::new(1.0, 0.05, 0.05);

    // 1. Mid-focus viewport resize: 1600x900 -> 500x1000 (portrait).
    let mut app = build_navigation_test_app();
    let cam_entity = spawn_test_camera(
        &mut app,
        ArcballCamera {
            yaw: 0.0,
            elevation: 0.0,
            distance: 10.0,
            zoom_target: 10.0,
            ..Default::default()
        },
        1600,
        900,
    );
    app.update();

    let intent = FramingIntent::Bounds { min, max };
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: Vec3::ZERO,
        start_distance: 10.0,
        target_focus: Vec3::ZERO,
        target_distance: 1.5, // initial landscape target
        framing_intent: Some(intent),
        duration: 0.4,
        remaining: 0.4,
        ..Default::default()
    };

    // Resize viewport to narrow portrait (500x1000, aspect 0.5) mid-tween.
    app.world_mut()
        .get_mut::<Camera>(cam_entity)
        .expect("cam")
        .computed
        .target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(500, 1000),
        scale_factor: 1.0,
    });
    if let Projection::Perspective(p) = app
        .world_mut()
        .get_mut::<Projection>(cam_entity)
        .expect("proj")
        .as_mut()
    {
        p.aspect_ratio = 0.5;
    }

    // Complete transition and verify all 8 corners fit inside 500x1000 portrait NDC.
    app.world_mut().resource_mut::<FlyTo>().duration = 0.0;
    app.update();
    refresh_computed_projection(&mut app, cam_entity);

    let tr = *app.world().get::<Transform>(cam_entity).expect("tr");
    let Projection::Perspective(proj) = app.world().get::<Projection>(cam_entity).expect("proj")
    else {
        panic!("expected perspective");
    };
    let view_from_world = tr.compute_affine().inverse();
    let clip_from_view = proj.get_clip_from_view();
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let view = view_from_world.transform_point3(corner);
        let clip = clip_from_view * view.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() <= 1.0 + 1.0e-4 && ndc.y.abs() <= 1.0 + 1.0e-4,
            "mid-tween resized corner {corner:?} cropped in portrait NDC: {ndc:?}"
        );
    }

    // 1b. Mid-focus FOV narrowing (45° -> 30°): final corners must remain inside NDC.
    let dist_at_45 = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("cam")
        .distance;
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: Vec3::ZERO,
        start_distance: dist_at_45,
        target_focus: Vec3::ZERO,
        target_distance: dist_at_45,
        framing_intent: Some(intent),
        duration: 0.4,
        remaining: 0.4,
        ..Default::default()
    };
    if let Projection::Perspective(p) = app
        .world_mut()
        .get_mut::<Projection>(cam_entity)
        .expect("proj")
        .as_mut()
    {
        p.fov = 30.0_f32.to_radians();
    }
    app.world_mut().resource_mut::<FlyTo>().duration = 0.0;
    app.update();
    refresh_computed_projection(&mut app, cam_entity);
    let dist_at_30 = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("cam")
        .distance;
    assert!(
        dist_at_30 > dist_at_45 * 1.4,
        "narrowing FOV 45->30 mid-tween must pull camera farther back: d45={dist_at_45}, d30={dist_at_30}"
    );

    // 2. Mid-focus projection switch: Perspective -> Orthographic.
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: Vec3::ZERO,
        start_distance: 10.0,
        target_focus: Vec3::ZERO,
        target_distance: 5.0,
        framing_intent: Some(intent),
        duration: 0.4,
        remaining: 0.4,
        ..Default::default()
    };
    let mut ortho = OrthographicProjection {
        scaling_mode: ScalingMode::WindowSize,
        ..OrthographicProjection::default_3d()
    };
    ortho.update(500.0, 1000.0);
    *app.world_mut()
        .get_mut::<Projection>(cam_entity)
        .expect("proj") = Projection::Orthographic(ortho);
    app.world_mut().resource_mut::<FlyTo>().duration = 0.0;
    app.update();

    let tr_ortho = *app.world().get::<Transform>(cam_entity).expect("tr");
    let Projection::Orthographic(final_ortho) =
        app.world().get::<Projection>(cam_entity).expect("proj")
    else {
        panic!("expected ortho");
    };
    let v_ortho = tr_ortho.compute_affine().inverse();
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let view = v_ortho.transform_point3(corner);
        assert!(
            view.x >= final_ortho.area.min.x - 1.0e-3
                && view.x <= final_ortho.area.max.x + 1.0e-3
                && view.y >= final_ortho.area.min.y - 1.0e-3
                && view.y <= final_ortho.area.max.y + 1.0e-3
        );
    }

    // 3. Mid-focus zero viewport cancels cleanly preserving last valid pose.
    let saved_focus = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("cam")
        .focus;
    *app.world_mut().resource_mut::<FlyTo>() = FlyTo {
        start_focus: saved_focus,
        start_distance: 5.0,
        target_focus: Vec3::new(50.0, 0.0, 0.0),
        target_distance: 2.0,
        framing_intent: Some(intent),
        duration: 0.4,
        remaining: 0.4,
        ..Default::default()
    };
    app.world_mut()
        .get_mut::<Camera>(cam_entity)
        .expect("cam")
        .computed
        .target_info = Some(RenderTargetInfo {
        physical_size: UVec2::ZERO,
        scale_factor: 1.0,
    });
    app.update();
    assert_eq!(app.world().resource::<FlyTo>().remaining, 0.0);
    assert_eq!(app.world().resource::<FlyTo>().framing_intent, None);
    assert_eq!(
        app.world()
            .get::<ArcballCamera>(cam_entity)
            .expect("cam")
            .focus,
        saved_focus
    );
}
