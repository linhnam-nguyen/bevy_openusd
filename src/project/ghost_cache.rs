//! Persistent render-blob references used by historical diff ghosts.
//!
//! Semantic extraction stays renderer-neutral. This small application-side
//! adapter runs after the current Bevy projection exists and attaches the
//! content address of a projected mesh to the corresponding semantic entity.
//! The scene overlay can hydrate that blob without opening another `LiveStage`.

use std::collections::HashMap;

use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::prelude::{Resource, World};
use usd_bevy::UsdPrimRef;
use usd_model::SemanticSnapshot;

use super::blob_store::{FilesystemBlobStore, OBJECTS_DIRECTORY, put_mesh};
use super::recovery::RecoverySettings;

/// Runtime counters for historical geometry capture.
///
/// The resource is intentionally a small diagnostics surface. The frontend
/// must receive a future read-model projection rather than inspecting this
/// Bevy resource directly.
#[derive(Debug, Default, Resource)]
pub(crate) struct HistoricalGeometryCache {
    pub(crate) snapshots_seen: u64,
    pub(crate) blob_references_attached: u64,
    pub(crate) capture_failures: u64,
    pub(crate) ghost_mesh_hydrations: u64,
    pub(crate) ghost_load_failures: u64,
}

/// Attach persistent render-blob identities to the semantic snapshot's mesh
/// entities. Existing references are retained because they belong to the
/// extracted historical/current snapshot already being synchronized.
pub(crate) fn attach_render_blobs(world: &mut World, snapshot: &mut SemanticSnapshot) {
    let Some(project_root) = world
        .get_resource::<RecoverySettings>()
        .map(|settings| settings.project_root.clone())
    else {
        return;
    };
    let Some(mesh_handles) = collect_mesh_handles(world) else {
        return;
    };
    let Some(meshes) = world.get_resource::<Assets<Mesh>>() else {
        return;
    };

    let store = match FilesystemBlobStore::new(project_root.join(OBJECTS_DIRECTORY)) {
        Ok(store) => store,
        Err(error) => {
            bevy::log::error!("[ghost-cache] cannot create mesh blob store: {error:#}");
            return;
        }
    };

    let mut captured = HashMap::new();
    let mut failures = 0;
    for (path, handle) in mesh_handles {
        let Some(mesh) = meshes.get(&handle) else {
            continue;
        };
        match put_mesh(&store, mesh) {
            Ok(blob_id) => {
                captured.entry(path).or_insert(blob_id);
            }
            Err(error) => {
                failures += 1;
                bevy::log::debug!(
                    "[ghost-cache] mesh at {} is not blob-serializable: {error:#}",
                    handle.id()
                );
            }
        }
    }
    let mut attached = 0;
    for entity in snapshot.entities.values_mut() {
        let Some(geometry) = entity.geometry.as_mut() else {
            continue;
        };
        if geometry.render_blob.is_some() {
            continue;
        }
        if let Some(blob_id) = captured.get(&entity.prim_path) {
            geometry.render_blob = Some(blob_id.clone());
            attached += 1;
        }
    }

    if let Some(mut cache) = world.get_resource_mut::<HistoricalGeometryCache>() {
        cache.snapshots_seen += 1;
        cache.blob_references_attached += attached;
        cache.capture_failures += failures;
    }
}

fn collect_mesh_handles(world: &mut World) -> Option<HashMap<String, bevy::asset::Handle<Mesh>>> {
    let mut query = world.query::<(&UsdPrimRef, &Mesh3d)>();
    let handles = query
        .iter(world)
        .map(|(prim, mesh)| (prim.path.clone(), mesh.0.clone()))
        .collect();
    Some(handles)
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
    use bevy::prelude::Assets;
    use tempfile::tempdir;
    use usd_model::{
        Bounds3, CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
        IdentitySource, QuantizedPoint3, SemanticInfo, SemanticProperty, SnapshotId,
        SnapshotSource, TransformSignature,
    };

    use crate::project::blob_store::BlobStore;

    use super::*;

    fn digest() -> HashDigest {
        HashDigest::new([7; HashDigest::BYTE_LEN])
    }

    fn snapshot() -> SemanticSnapshot {
        let path = "/World/Triangle".to_owned();
        let key = EntityKey::from(path.clone());
        let entity = EntitySnapshot {
            key: key.clone(),
            prim_path: path,
            identity_source: IdentitySource::PrimPath,
            semantic: SemanticInfo::default(),
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0; 4],
                scale_quantized: [10_000; 3],
                hash: digest(),
            },
            geometry: Some(GeometrySignature {
                vertex_count: 3,
                index_count: 3,
                local_bounds: Bounds3 {
                    min: [0.0; 3],
                    max: [1.0; 3],
                },
                local_centroid: QuantizedPoint3([500; 3]),
                topology_hash: digest(),
                shape_hash: digest(),
                render_blob: None,
            }),
            properties: vec![SemanticProperty {
                name: "test".to_owned(),
                value: CanonicalValue::Bool(true),
            }],
            metadata_hash: digest(),
            full_hash: digest(),
        };
        SemanticSnapshot {
            snapshot_id: SnapshotId("test-snapshot".to_owned()),
            source: SnapshotSource::Working {
                session: "test".to_owned(),
                live_revision: 1,
            },
            config_hash: digest(),
            entities: [(key, entity)].into_iter().collect(),
        }
    }

    #[test]
    fn projected_triangle_gets_a_persistent_blob_reference() -> anyhow::Result<()> {
        let project = tempdir()?;
        let mut world = World::new();
        world.insert_resource(RecoverySettings {
            project_root: project.path().to_path_buf(),
        });
        world.insert_resource(HistoricalGeometryCache::default());
        world.insert_resource(Assets::<Mesh>::default());

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((UsdPrimRef::new("/World/Triangle"), Mesh3d(handle)));

        let mut snapshot = snapshot();
        attach_render_blobs(&mut world, &mut snapshot);

        let blob_id = snapshot
            .entities
            .get(&EntityKey::from("/World/Triangle"))
            .and_then(|entity| entity.geometry.as_ref())
            .and_then(|geometry| geometry.render_blob.as_ref())
            .cloned()
            .expect("projected mesh should have a blob reference");
        let store = FilesystemBlobStore::new(project.path().join(OBJECTS_DIRECTORY))?;
        assert!(store.contains(&blob_id)?);
        assert_eq!(
            world.resource::<HistoricalGeometryCache>().snapshots_seen,
            1
        );
        assert_eq!(
            world
                .resource::<HistoricalGeometryCache>()
                .blob_references_attached,
            1
        );
        Ok(())
    }
}
