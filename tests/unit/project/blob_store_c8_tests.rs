use std::fs;

use super::*;

#[test]
fn putting_the_original_payload_repairs_a_corrupt_object() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
    let payload = b"repairable-payload";
    let id = store.put(payload)?;
    fs::write(store.object_path(&id)?, b"corrupt")?;

    assert!(store.get(&id).is_err());
    assert_eq!(store.put(payload)?, id);
    assert_eq!(store.get(&id)?.as_deref(), Some(payload.as_slice()));
    Ok(())
}
