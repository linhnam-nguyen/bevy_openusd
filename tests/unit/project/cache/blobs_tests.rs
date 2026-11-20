use std::fs;
use std::path::Path;

use super::*;

#[test]
fn filesystem_store_is_content_addressed_and_idempotent() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join(".usdhub/cache/objects"))?;

    let first = store.put(b"mesh-payload")?;
    let second = store.put(b"mesh-payload")?;
    let other = store.put(b"different-payload")?;

    assert_eq!(first, second);
    assert_ne!(first, other);
    assert_eq!(
        store.get(&first)?.as_deref(),
        Some(b"mesh-payload".as_slice())
    );
    assert!(store.contains(&first)?);

    let object = store.object_path(&first)?;
    assert!(object.is_file());
    assert_eq!(
        object.parent().and_then(Path::file_name),
        Some(std::ffi::OsStr::new(&first.0[..2]))
    );
    Ok(())
}

#[test]
fn corrupt_blob_is_detected() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let id = store.put(b"valid-payload")?;
    fs::write(store.object_path(&id)?, b"corrupt")?;

    let error = store
        .get(&id)
        .expect_err("corrupt object should be rejected");
    assert!(error.to_string().contains("digest mismatch"));
    Ok(())
}

#[test]
fn mesh_blob_round_trips() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));

    let expected = MeshBlob::from_bevy_mesh(&mesh)?;
    let id = put_mesh(&store, &mesh)?;
    let updated_bytes = store.get(&id)?.unwrap();
    assert!(
        updated_bytes.starts_with(b"NMSH"),
        "new mesh writes use a distinct binary format"
    );
    let restored = get_mesh(&store, &id)?.expect("mesh blob should exist");
    assert_eq!(expected, MeshBlob::from_bevy_mesh(&restored)?);

    let mut legacy = expected.clone();
    legacy.version = 1;
    let old_bytes = serde_json::to_vec(&legacy)?;
    let old_id = store.put(&old_bytes)?;
    let old_mesh = get_mesh(&store, &old_id)?.expect("legacy mesh is still readable");
    assert_eq!(expected, MeshBlob::from_bevy_mesh(&old_mesh)?);
    assert_ne!(id, old_id);
    assert_eq!(updated_bytes.len(), 172);
    assert!(updated_bytes.len() < old_bytes.len());
    println!(
        "mesh fixture cache hits: 2/2, v1={} bytes, v2={} bytes",
        old_bytes.len(),
        updated_bytes.len()
    );

    legacy.indices[2] = 99;
    let malformed_index = store.put(&serde_json::to_vec(&legacy)?)?;
    assert!(get_mesh(&store, &malformed_index).is_err());
    let mut oversized = updated_bytes[..16].to_vec();
    oversized[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(get_mesh(&store, &store.put(&oversized)?).is_err());
    let truncated = store.put(&updated_bytes[..updated_bytes.len() - 1])?;
    assert!(get_mesh(&store, &truncated).is_err());
    assert_eq!(store.get(&old_id)?.unwrap(), old_bytes);
    Ok(())
}

#[test]
fn invalid_blob_id_cannot_escape_store_root() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let error = store
        .get(&BlobId("../outside".to_owned()))
        .expect_err("invalid id");
    assert!(error.to_string().contains("invalid blob id"));
    Ok(())
}
