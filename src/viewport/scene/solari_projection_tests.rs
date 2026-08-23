#[cfg(feature = "solari")]
use super::*;
#[cfg(feature = "solari")]
use crate::viewport::scene::visualization::DisplayToggles;
#[cfg(feature = "solari")]
use bevy::mesh::PrimitiveTopology;
#[cfg(feature = "solari")]
use usd_bevy::UsdPrimRef;

#[cfg(feature = "solari")]
#[test]
fn supported_usd_meshes_get_raytracing_markers_without_material_mutation() {
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
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/World/Supported"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
        ))
        .id();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    })
    .init_resource::<SolariProjectionState>()
    .init_resource::<SolariProjectionDiagnostics>()
    .insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            render_mode: viewport_protocol::RenderMode::RayTraced,
            ..Default::default()
        },
        ..Default::default()
    })
    .add_systems(Update, sync_solari_usd_projection);

    app.update();

    assert_eq!(app.world().get::<Mesh3d>(entity).unwrap().0, mesh);
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0,
        material
    );
    assert_eq!(
        app.world()
            .get::<bevy::solari::prelude::RaytracingMesh3d>(entity)
            .unwrap()
            .0,
        mesh
    );
}

#[cfg(feature = "solari")]
#[test]
fn unsupported_usd_projection_is_diagnosed_and_never_marked_for_raytracing() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::new(
            PrimitiveTopology::PointList,
            bevy::asset::RenderAssetUsages::MAIN_WORLD,
        ));
    let entity = app
        .world_mut()
        .spawn((UsdPrimRef::new("/World/Points"), Mesh3d(mesh)))
        .id();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    })
    .init_resource::<SolariProjectionState>()
    .insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            render_mode: viewport_protocol::RenderMode::RayTraced,
            ..Default::default()
        },
        ..Default::default()
    })
    .init_resource::<SolariProjectionDiagnostics>()
    .add_systems(Update, sync_solari_usd_projection);

    app.update();

    assert!(
        app.world()
            .get::<bevy::solari::prelude::RaytracingMesh3d>(entity)
            .is_none()
    );
    assert_eq!(
        app.world()
            .resource::<SolariProjectionDiagnostics>()
            .unsupported_meshes,
        1
    );
    assert!(!app.world().resource::<SolariCapability>().scene_eligible);
}

#[cfg(feature = "solari")]
#[test]
fn shaded_idle_does_not_repeat_full_projection_scan() {
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
    app.world_mut().spawn((
        UsdPrimRef::new("/World/Idle"),
        Mesh3d(mesh),
        MeshMaterial3d(material),
    ));
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: false,
    })
    .insert_resource(DisplayToggles::default())
    .init_resource::<SolariProjectionDiagnostics>()
    .init_resource::<SolariProjectionState>()
    .init_resource::<SolariProjectionStats>()
    .add_systems(Update, sync_solari_usd_projection);

    app.update();
    assert_eq!(
        app.world().resource::<SolariProjectionStats>().full_scans,
        1
    );
    app.update();

    let stats = app.world().resource::<SolariProjectionStats>();
    assert_eq!(stats.full_scans, 1);
    assert_eq!(stats.incremental_entities, 0);
}

#[cfg(feature = "solari")]
#[test]
fn stable_ray_traced_does_not_repeat_full_projection_scan() {
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
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/World/RayTraced"),
            Mesh3d(mesh),
            MeshMaterial3d(material),
        ))
        .id();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: false,
    })
    .insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            render_mode: viewport_protocol::RenderMode::RayTraced,
            ..Default::default()
        },
        ..Default::default()
    })
    .init_resource::<SolariProjectionDiagnostics>()
    .init_resource::<SolariProjectionState>()
    .init_resource::<SolariProjectionStats>()
    .add_systems(Update, sync_solari_usd_projection);

    app.update();
    assert!(
        app.world()
            .get::<bevy::solari::prelude::RaytracingMesh3d>(entity)
            .is_some()
    );
    app.update();

    let stats = app.world().resource::<SolariProjectionStats>();
    assert_eq!(stats.full_scans, 1);
    assert_eq!(stats.incremental_entities, 0);
}
