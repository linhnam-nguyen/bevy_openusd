use super::*;

#[test]
fn solari_capability_requires_compile_device_and_scene_support() {
    let mut capability = SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    };
    assert!(capability.supported());

    capability.scene_eligible = false;
    assert!(!capability.supported());
    capability.scene_eligible = true;
    capability.device_supported = false;
    assert!(!capability.supported());
    capability.device_supported = true;
    capability.compiled = false;
    assert!(!capability.supported());
}

#[test]
fn supported_capability_is_published_as_renderer_neutral_viewer_state() {
    let mut app = App::new();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    })
    .init_resource::<ViewerSettingsState>()
    .add_systems(Update, publish_capability);

    app.update();

    assert!(
        app.world()
            .resource::<ViewerSettingsState>()
            .ray_traced_supported()
    );
}
