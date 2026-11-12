use std::collections::HashMap;

use tempfile::tempdir;
use usd_model::{
    CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest, IdentitySource,
    QuantizedPoint3, SemanticInfo, SemanticProperty, SnapshotId, SnapshotSource,
    TransformSignature,
};

use super::*;
use crate::project::blob_store::FilesystemBlobStore;

fn digest() -> HashDigest {
    HashDigest::new([3; HashDigest::BYTE_LEN])
}

fn snapshot(mesh_blob: Option<usd_model::BlobId>) -> SemanticSnapshot {
    let key = EntityKey::from("/World/Box");
    SemanticSnapshot {
        snapshot_id: SnapshotId("working-7".to_owned()),
        source: SnapshotSource::Working {
            session: "test".to_owned(),
            live_revision: 7,
        },
        config_hash: digest(),
        entities: HashMap::from([(
            key.clone(),
            EntitySnapshot {
                key,
                prim_path: "/World/Box".to_owned(),
                identity_source: IdentitySource::PrimPath,
                semantic: SemanticInfo::default(),
                transform: TransformSignature {
                    translation_mm: [0, 0, 0],
                    rotation_quantized: [0, 0, 0, 10_000],
                    scale_quantized: [10_000; 3],
                    hash: digest(),
                },
                geometry: mesh_blob.map(|render_blob| GeometrySignature {
                    vertex_count: 3,
                    index_count: 3,
                    local_bounds: Bounds3 {
                        min: [0.0; 3],
                        max: [1.0; 3],
                    },
                    local_centroid: QuantizedPoint3([500; 3]),
                    topology_hash: digest(),
                    shape_hash: digest(),
                    render_blob: Some(render_blob),
                }),
                properties: vec![SemanticProperty {
                    name: "secret".to_owned(),
                    value: CanonicalValue::Bool(true),
                    measurement: None,
                    display_name: None,
                }],
                metadata_hash: digest(),
                full_hash: digest(),
            },
        )]),
    }
}

#[test]
fn bundle_reuses_mesh_bytes_and_excludes_semantic_properties() -> Result<()> {
    let directory = tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let mesh_id = store.put(b"mesh-bytes")?;
    let bundle = build_runtime_delivery(
        &store,
        &snapshot(Some(mesh_id.clone())),
        RuntimeProfile::NativeMedium,
    )?;

    assert_eq!(bundle.manifest.meshes.len(), 1);
    assert_eq!(bundle.blobs.len(), 2);
    assert!(bundle.manifest.hierarchy.byte_size > 0);
    assert!(!bundle.blobs.iter().any(|(_, bytes)| {
        bytes
            .windows(b"secret".len())
            .any(|window| window == b"secret")
    }));
    assert_eq!(bundle.blobs[1], (mesh_id.0, b"mesh-bytes".to_vec()));
    Ok(())
}

#[test]
fn missing_mesh_payload_stops_publication() -> Result<()> {
    let directory = tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let missing = usd_model::BlobId(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
    );

    let error = build_runtime_delivery(
        &store,
        &snapshot(Some(missing)),
        RuntimeProfile::NativeMedium,
    )
    .expect_err("missing mesh should not produce a partial manifest");
    assert!(error.to_string().contains("missing from the BlobStore"));
    Ok(())
}

#[test]
fn empty_snapshot_still_has_a_hierarchy_payload() -> Result<()> {
    let directory = tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let bundle = build_runtime_delivery(&store, &snapshot(None), RuntimeProfile::NativeMedium)?;
    assert!(bundle.manifest.meshes.is_empty());
    assert_eq!(bundle.blobs.len(), 1);
    Ok(())
}

#[test]
fn hierarchy_payload_preserves_semantic_display_name_without_using_raw_path() -> Result<()> {
    let directory = tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let mut semantic_snapshot = snapshot(None);
    semantic_snapshot
        .entities
        .get_mut(&EntityKey::from("/World/Box"))
        .expect("fixture entity")
        .semantic
        .display_name = Some("Friendly Box".to_owned());

    let bundle = build_runtime_delivery(&store, &semantic_snapshot, RuntimeProfile::NativeMedium)?;
    let hierarchy: RuntimeHierarchyBlob = serde_json::from_slice(&bundle.blobs[0].1)?;
    assert_eq!(hierarchy.entities[0].prim_path, "/World/Box");
    assert_eq!(
        hierarchy.entities[0].display_name.as_deref(),
        Some("Friendly Box")
    );
    Ok(())
}
