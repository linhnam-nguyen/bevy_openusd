use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use tempfile::tempdir;
use usd_model::{
    Bounds3, EntitySnapshot, GeometrySignature, HashDigest, IdentitySource, QuantizedPoint3,
    SemanticInfo, SemanticSnapshot, SnapshotSource, TransformSignature,
};

use super::*;
use crate::project::blob_store::put_mesh;

fn digest() -> HashDigest {
    HashDigest::new([9; HashDigest::BYTE_LEN])
}

fn mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    mesh
}

fn snapshot(blob_id: Option<BlobId>, path: &str) -> SemanticSnapshot {
    let key = EntityKey::from(path);
    let entity = EntitySnapshot {
        key: key.clone(),
        prim_path: path.to_owned(),
        identity_source: IdentitySource::PrimPath,
        semantic: SemanticInfo::default(),
        transform: TransformSignature {
            translation_mm: [1_000, 0, 0],
            rotation_quantized: [0, 0, 0, 0],
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
            render_blob: blob_id,
        }),
        properties: Vec::new(),
        metadata_hash: digest(),
        full_hash: digest(),
    };
    SemanticSnapshot {
        snapshot_id: SnapshotId(path.to_owned()),
        source: SnapshotSource::Working {
            session: "ghost-test".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(),
        entities: [(key, entity)].into_iter().collect(),
    }
}

#[test]
fn historical_ghost_is_hydrated_reused_and_removed_with_the_diff() -> anyhow::Result<()> {
    let project = tempdir()?;
    let store = FilesystemBlobStore::new(project.path().join(OBJECTS_DIRECTORY))?;
    let blob_id = put_mesh(&store, &mesh())?;

    let baseline = snapshot(Some(blob_id.clone()), "/World/Triangle");
    let working = SemanticSnapshot {
        snapshot_id: SnapshotId("working".to_owned()),
        entities: Default::default(),
        ..baseline.clone()
    };
    let mut diff = SemanticDiffState::default();
    diff.update_working(1, baseline);
    assert!(diff.capture_baseline());
    diff.update_working(1, working);

    let mut app = App::new();
    app.insert_resource(diff)
        .insert_resource(RecoverySettings {
            project_root: project.path().to_path_buf(),
        })
        .insert_resource(HistoricalGeometryCache::default())
        .init_resource::<HistoricalGhostState>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_systems(Update, hydrate_historical_ghosts);

    app.update();
    let first = {
        let mut query = app
            .world_mut()
            .query::<(&HistoricalGhost, &Mesh3d, &Transform)>();
        query
            .iter(app.world())
            .map(|(ghost, _, _)| ghost.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].blob_id, blob_id);
    assert_eq!(
        app.world()
            .resource::<HistoricalGeometryCache>()
            .ghost_mesh_hydrations,
        1
    );

    app.update();
    let second_count = {
        let mut query = app.world_mut().query::<&HistoricalGhost>();
        query.iter(app.world()).count()
    };
    assert_eq!(second_count, 1);

    app.world_mut()
        .resource_mut::<SemanticDiffState>()
        .clear_baseline();
    app.update();
    let final_count = {
        let mut query = app.world_mut().query::<&HistoricalGhost>();
        query.iter(app.world()).count()
    };
    assert_eq!(final_count, 0);
    Ok(())
}
