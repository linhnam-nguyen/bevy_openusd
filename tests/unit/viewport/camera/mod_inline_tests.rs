use super::*;

#[test]
fn pole_rig_uses_a_stable_up_axis() {
    let cam = ArcballCamera {
        elevation: core::f32::consts::FRAC_PI_2,
        ..Default::default()
    };
    let mut transform = Transform::default();
    apply_rig(&cam, &mut transform);
    assert!(transform.rotation.is_finite());
}

#[test]
fn orientation_publisher_emits_initial_orientation_once_and_then_stays_idle() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<CameraOrientationState>()
        .init_resource::<ViewportEventOutbox>()
        .add_systems(Update, publish_camera_orientation);
    app.world_mut()
        .spawn((Camera3d::default(), Transform::default()));

    app.update();
    let first = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("the initial live camera orientation is published");
    assert!(matches!(
        first.event,
        ViewportEvent::CameraOrientationChanged { orientation }
            if orientation == CameraOrientationReadModel::default()
    ));

    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .is_none()
    );
}

#[test]
fn orientation_publisher_drops_nonfinite_camera_rotations() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<CameraOrientationState>()
        .init_resource::<ViewportEventOutbox>()
        .add_systems(Update, publish_camera_orientation);
    app.world_mut().spawn((
        Camera3d::default(),
        Transform {
            rotation: Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0),
            ..Default::default()
        },
    ));

    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .is_none()
    );
}

#[test]
fn pan_orbit_camera_preserves_focus_on_scroll_and_respects_ui_capture() {
    use bevy::camera::{CameraProjection, Projection, RenderTargetInfo};
    use bevy::camera_controller::pan_orbit_camera::controller::MinimalPanOrbitCameraPlugin;
    use crate::viewport::navigation::input::{HostUiInputCapture, ViewportNavigationInput};

    let mut app = App::new();
    app.add_message::<bevy::window::RequestRedraw>();
    app.add_plugins((MinimalPlugins, MinimalPanOrbitCameraPlugin));
    app.init_resource::<CameraMount>()
        .init_resource::<CameraNavigationConfig>()
        .init_resource::<ViewportNavigationInput>()
        .init_resource::<HostUiInputCapture>()
        .add_systems(
            PreUpdate,
            sync_arcball_from_pan_orbit
                .after(bevy::camera_controller::pan_orbit_camera::SyncCameraPosition),
        )
        .add_systems(
            Update,
            drive_pan_orbit_camera.run_if(arcball_is_active),
        );

    let initial_arcball = ArcballCamera {
        focus: Vec3::new(1.0, 2.0, -3.0),
        distance: 10.0,
        ..Default::default()
    };
    let mut initial_transform = Transform::default();
    apply_rig(&initial_arcball, &mut initial_transform);

    let projection = Projection::Perspective(PerspectiveProjection {
        aspect_ratio: 1280.0 / 720.0,
        ..default()
    });
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    camera.computed.clip_from_view = projection.get_clip_from_view();
    let mut pan_orbit = default_pan_orbit_camera(initial_arcball.distance);
    pan_orbit.smoothing.zoom = std::time::Duration::ZERO;
    let cam_entity = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            camera,
            projection,
            initial_transform,
            pan_orbit,
            initial_arcball,
        ))
        .id();

    app.update();

    // 1. Focus-centered scroll: zooming in must reduce distance while keeping focus invariant.
    {
        let mut input = app.world_mut().resource_mut::<ViewportNavigationInput>();
        input.focused = true;
        input.viewport_size = Vec2::new(1280.0, 720.0);
        input.wheel_delta = Vec2::new(0.0, 2.0);
    }
    app.update();
    app.world_mut()
        .resource_mut::<ViewportNavigationInput>()
        .wheel_delta = Vec2::ZERO;
    app.update();
    let after_zoom = app.world().get::<ArcballCamera>(cam_entity).unwrap();
    assert!(after_zoom.distance < 10.0, "scroll up must zoom closer");
    assert!(
        (after_zoom.focus - Vec3::new(1.0, 2.0, -3.0)).length() < 1e-3,
        "focus must remain invariant during scroll: {:?}",
        after_zoom.focus
    );
    let dist_after_zoom = after_zoom.distance;

    // 2. UI pointer capture must block camera motion.
    app.world_mut().resource_mut::<HostUiInputCapture>().wants_pointer = true;
    {
        let mut input = app.world_mut().resource_mut::<ViewportNavigationInput>();
        input.wheel_delta = Vec2::new(0.0, 5.0);
    }
    app.update();
    app.update();
    let after_blocked = app.world().get::<ArcballCamera>(cam_entity).unwrap();
    assert!(
        (after_blocked.distance - dist_after_zoom).abs() < 0.05,
        "UI pointer capture must prevent scroll zoom"
    );

    // 3. Orbit + Pan via remote/native ViewportNavigationInput.
    app.world_mut().resource_mut::<HostUiInputCapture>().wants_pointer = false;
    {
        let mut input = app.world_mut().resource_mut::<ViewportNavigationInput>();
        input.wheel_delta = Vec2::ZERO;
        input.buttons.primary = true;
        input.buttons.secondary = true;
        input.pointer_delta = Vec2::new(40.0, 20.0);
    }
    app.update();
    app.update();
    let after_orbit = app.world().get::<ArcballCamera>(cam_entity).unwrap();
    assert!((after_orbit.yaw - 0.0).abs() > 1e-3 || (after_orbit.elevation - 25f32.to_radians()).abs() > 1e-3);
}
