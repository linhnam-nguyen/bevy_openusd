use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::Assets;
use tempfile::tempdir;
use usd_model::{
    Bounds3, CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
    IdentitySource, QuantizedPoint3, SemanticInfo, SemanticProperty, SnapshotId, SnapshotSource,
    TransformSignature,
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
    assert_eq!(
        world
            .resource::<HistoricalGeometryCache>()
            .mesh_handles_scanned,
        1
    );
    assert_eq!(
        world
            .resource::<HistoricalGeometryCache>()
            .semantic_entities_scanned,
        1
    );
    Ok(())
}

#[test]
fn profiles_render_blob_baseline_scans_all_meshes_and_entities() -> anyhow::Result<()> {
    let project = tempdir()?;
    let mut world = World::new();
    world.insert_resource(RecoverySettings {
        project_root: project.path().to_path_buf(),
    });
    world.insert_resource(HistoricalGeometryCache::default());
    world.insert_resource(Assets::<Mesh>::default());

    // Spawn 3 meshes
    for path in ["/World/Mesh1", "/World/Mesh2", "/World/Mesh3"] {
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
        world.spawn((UsdPrimRef::new(path), Mesh3d(handle)));
    }

    // Build snapshot with 10 entities (only Mesh1 matches geometry)
    let mut entities = std::collections::HashMap::new();
    for i in 0..10 {
        let path = if i == 0 {
            "/World/Mesh1".to_owned()
        } else {
            format!("/World/Other{i}")
        };
        let key = EntityKey::from(path.clone());
        let entity = EntitySnapshot {
            key: key.clone(),
            prim_path: path.clone(),
            identity_source: IdentitySource::PrimPath,
            semantic: SemanticInfo::default(),
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0; 4],
                scale_quantized: [10_000; 3],
                hash: digest(),
            },
            geometry: (i == 0).then_some(GeometrySignature {
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
            properties: Vec::new(),
            metadata_hash: digest(),
            full_hash: digest(),
        };
        entities.insert(key, entity);
    }

    let mut snap = SemanticSnapshot {
        snapshot_id: SnapshotId("baseline-blob-test".to_owned()),
        source: SnapshotSource::Working {
            session: "test".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(),
        entities,
    };

    attach_render_blobs(&mut world, &mut snap);

    let cache = *world.resource::<HistoricalGeometryCache>();
    assert_eq!(cache.snapshots_seen, 1);
    assert_eq!(cache.mesh_handles_scanned, 3);
    assert_eq!(cache.semantic_entities_scanned, 10);
    assert_eq!(cache.blob_references_attached, 1);
    assert_eq!(cache.capture_failures, 0);
    Ok(())
}

#[test]
fn attach_render_blobs_for_entities_scans_only_affected_entities_and_handles() -> anyhow::Result<()>
{
    let project = tempdir()?;
    let mut world = World::new();
    world.insert_resource(RecoverySettings {
        project_root: project.path().to_path_buf(),
    });
    world.insert_resource(HistoricalGeometryCache::default());
    world.insert_resource(Assets::<Mesh>::default());

    let mut prim_entities = usd_bevy::PrimEntities::default();

    // Spawn 3 meshes in Bevy
    for path in ["/World/Mesh1", "/World/Mesh2", "/World/Mesh3"] {
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
        let entity = world.spawn((UsdPrimRef::new(path), Mesh3d(handle))).id();
        prim_entities.insert(path, entity);
    }
    world.insert_resource(prim_entities);

    // Only 1 affected upsert entity (/World/Mesh1)
    let mut upserts = vec![EntitySnapshot {
        key: EntityKey::from("/World/Mesh1"),
        prim_path: "/World/Mesh1".to_owned(),
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
        properties: Vec::new(),
        metadata_hash: digest(),
        full_hash: digest(),
    }];

    super::attach_render_blobs_for_entities(&mut world, &mut upserts);

    let cache = *world.resource::<HistoricalGeometryCache>();
    assert_eq!(cache.snapshots_seen, 1);
    // Scanned ONLY 1 mesh handle and 1 semantic entity instead of the whole world!
    assert_eq!(cache.mesh_handles_scanned, 1);
    assert_eq!(cache.semantic_entities_scanned, 1);
    assert_eq!(cache.blob_references_attached, 1);
    assert_eq!(cache.capture_failures, 0);

    let blob_id = upserts[0]
        .geometry
        .as_ref()
        .and_then(|g| g.render_blob.as_ref())
        .expect("blob reference attached");
    let store = FilesystemBlobStore::new(project.path().join(OBJECTS_DIRECTORY))?;
    assert!(store.contains(blob_id)?);
    Ok(())
}
