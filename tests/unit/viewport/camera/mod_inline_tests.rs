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
