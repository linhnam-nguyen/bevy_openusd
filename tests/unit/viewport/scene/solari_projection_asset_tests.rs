#[cfg(feature = "solari")]
use super::*;
#[cfg(feature = "solari")]
use crate::viewport::scene::visualization::DisplayToggles;
#[cfg(feature = "solari")]
use bevy::asset::AssetEvent;
#[cfg(feature = "solari")]
use bevy::ecs::message::Messages;
#[cfg(feature = "solari")]
use bevy::mesh::Mesh3d;
#[cfg(feature = "solari")]
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
#[cfg(feature = "solari")]
use usd_bevy::{MeshProjectionConsumers, RenderProjectionDirtySet, UsdPrimRef};

#[cfg(feature = "solari")]
#[test]
fn mesh_asset_event_dirties_indexed_consumer() {
    let mut app = App::new();
    app.add_message::<AssetEvent<Mesh>>();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(0.5).mesh().build());
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/World/AssetEvent"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
        ))
        .id();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: false,
    })
    .insert_resource(DisplayToggles::default())
    .insert_resource(SolariProjectionState::initialized_for_test())
    .init_resource::<SolariProjectionDiagnostics>()
    .init_resource::<SolariProjectionStats>()
    .init_resource::<RenderProjectionDirtySet>()
    .init_resource::<MeshProjectionConsumers>()
    .add_systems(Update, sync_solari_usd_projection);
    app.world_mut()
        .resource_mut::<MeshProjectionConsumers>()
        .track(entity, mesh.id());
    app.world_mut()
        .resource_mut::<Messages<AssetEvent<Mesh>>>()
        .write(AssetEvent::Modified { id: mesh.id() });

    app.update();

    let stats = app.world().resource::<SolariProjectionStats>();
    assert_eq!(stats.full_scans, 0);
    assert_eq!(stats.incremental_entities, 1);
}
