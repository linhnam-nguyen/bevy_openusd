use std::fs;

use anyhow::Result;

use super::*;

#[test]
fn standalone_assembly_is_deterministic_and_preserves_source() -> Result<()> {
    let input = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    let root = input.path().join("source.usda");
    let dependency = input.path().join("building.usda");
    let root_bytes = br#"#usda 1.0
def Xform "World"
{
    def Xform "FloorA" (
        prepend references = @./building.usda@</Building>
        instanceable = true
    ) {}
    def Xform "FloorB" (
        prepend references = @./building.usda@</Building>
        instanceable = true
    ) {}
    def PointInstancer "Rack"
    {
        rel prototypes = </World/FloorA/Wall>
        int[] protoIndices = [0]
        point3f[] positions = [(0, 0, 0)]
        int64[] ids = [84]
    }
}
"#;
    let dependency_bytes = br#"#usda 1.0
def Xform "Building" (
    variants = { string lod = "high" }
    prepend variantSets = "lod"
)
{
    def Cube "Wall" { double size = 2 }
    variantSet "lod" = {
        "high" {}
        "low" {}
    }
}
"#;
    fs::write(&root, root_bytes)?;
    fs::write(&dependency, dependency_bytes)?;
    let first = prepare_and_publish(&root, cache.path())?;
    let second = prepare_and_publish(&root, cache.path())?;
    assert_eq!(first, second);
    let assembly_root = cache.path().join(ASSEMBLIES_DIRECTORY);
    let blobs = FilesystemBlobStore::new(assembly_root.join("objects"))?;
    let manifest = load(&assembly_root, &blobs, first)?.unwrap();
    assert_eq!(
        manifest.source_digest.to_hex(),
        source_closure_fingerprint(&root)?
    );
    assert_eq!(manifest.sources.len(), 2);
    assert_eq!(
        blobs
            .get(&manifest.sources["source/building.usda"])?
            .unwrap(),
        dependency_bytes
    );
    let floor: PartitionPayload =
        serde_json::from_slice(&blobs.get(&manifest.partitions["/World/FloorB"])?.unwrap())?;
    assert!(floor.bounds.is_some());
    assert_eq!(
        floor.occurrences[0]
            .authored_lod
            .get("lod")
            .map(String::as_str),
        Some("high")
    );
    assert_eq!(
        floor.occurrences[1].native_prototype_source.as_deref(),
        Some("/World/FloorA")
    );
    assert_eq!(
        floor.occurrences[1].native_prototype_member.as_deref(),
        Some("Wall")
    );
    let rack: PartitionPayload =
        serde_json::from_slice(&blobs.get(&manifest.partitions["/World/Rack"])?.unwrap())?;
    assert_eq!(rack.occurrences[0].point_instances[0].authored_id, Some(84));
    assert_eq!(
        rack.occurrences[0].point_instances[0].prototype_path,
        "/World/FloorA/Wall"
    );

    let mut candidate = manifest.clone();
    candidate
        .partitions
        .insert("/World/Rack".to_owned(), BlobId("00".repeat(32)));
    assert!(publish(&assembly_root, &blobs, &candidate).is_err());
    candidate.partitions.insert(
        "/World/Rack".to_owned(),
        blobs.put(b"invalid partition payload")?,
    );
    assert!(publish(&assembly_root, &blobs, &candidate).is_err());
    assert_eq!(load(&assembly_root, &blobs, first)?, Some(manifest));
    assert_eq!(fs::read(&root)?, root_bytes);
    assert_eq!(fs::read(&dependency)?, dependency_bytes);

    let changed_bytes = String::from_utf8_lossy(dependency_bytes).replace("size = 2", "size = 4");
    fs::write(&dependency, changed_bytes)?;
    let changed = prepare_and_publish(&root, cache.path())?;
    assert_ne!(changed, first);
    assert!(load(&assembly_root, &blobs, first)?.is_some());
    let historical = tempfile::tempdir()?;
    let historical_root = historical.path().join("source.usda");
    fs::write(&historical_root, root_bytes)?;
    fs::write(historical.path().join("building.usda"), dependency_bytes)?;
    assert_eq!(prepare_and_publish(&historical_root, cache.path())?, first);
    Ok(())
}
