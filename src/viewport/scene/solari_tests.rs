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

#[cfg(feature = "solari")]
#[test]
fn camera_and_proof_mesh_follow_ray_traced_mode_and_restore_shaded() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Sphere::new(0.5)
            .mesh()
            .build()
            .with_generated_tangents()
            .expect("test sphere must provide generated tangents"),
    );
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let camera = app.world_mut().spawn(Camera3d::default()).id();
    let proof_mesh = app
        .world_mut()
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            SolariProofMesh,
        ))
        .id();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    })
    .insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            render_mode: viewport_protocol::RenderMode::RayTraced,
            ..Default::default()
        },
        ..Default::default()
    })
    .add_systems(
        Update,
        (sync_solari_camera, sync_solari_proof_meshes).chain(),
    );

    app.update();
    assert!(
        app.world()
            .get::<bevy::solari::prelude::SolariLighting>(camera)
            .is_some()
    );
    assert_eq!(
        app.world()
            .get::<bevy::solari::prelude::RaytracingMesh3d>(proof_mesh)
            .map(|mesh| mesh.0.clone()),
        Some(mesh.clone())
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = viewport_protocol::RenderMode::Shaded;
    app.update();

    assert!(
        app.world()
            .get::<bevy::solari::prelude::SolariLighting>(camera)
            .is_none()
    );
    assert!(
        app.world()
            .get::<bevy::solari::prelude::RaytracingMesh3d>(proof_mesh)
            .is_none()
    );
}
