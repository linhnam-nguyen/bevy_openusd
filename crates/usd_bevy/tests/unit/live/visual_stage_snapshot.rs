use std::fs;
use std::io::Read;

use super::*;

#[test]
fn sealed_document_keeps_open_time_assets_and_replays_without_source_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("root.usda");
    let layer = directory.path().join("child.usda");
    let texture = directory.path().join("albedo.bin");
    let root_bytes = br#"#usda 1.0
(
    defaultPrim = "World"
    subLayers = [@./child.usda@]
)
def Xform "World" {
    asset userProperties:albedo = @./albedo.bin@
}
"#;
    let layer_bytes = b"#usda 1.0\ndef Xform \"Child\" {}\n";
    let texture_bytes = b"first-texture-revision";
    fs::write(&root, root_bytes)?;
    fs::write(&layer, layer_bytes)?;
    fs::write(&texture, texture_bytes)?;

    let (canonical, visual) = VisualStage::open_pair(&root).context("open document pair")?;
    assert!(canonical.prim(openusd::sdf::path("/World")?).is_ok());
    // Native all_dependencies must have prefetched even a lazily consumed
    // renderer asset before either context is made available to callers.
    let texture_path = fs::canonicalize(&texture)?;
    assert!(visual.source.borrow().contains_key(&texture_path));
    let texture_reader = visual.texture_source((1, 0));
    let resolver = SnapshotResolver {
        filesystem: DefaultResolver::new(),
        source: visual.source.clone(),
        revisions: HashMap::new(),
        sealed: true,
    };
    let resolved = resolver.resolve(texture_path.to_str().unwrap()).unwrap();
    let mut captured = Vec::new();
    std::io::Cursor::new(resolver.open_asset(&resolved)?.into_buffer()?)
        .read_to_end(&mut captured)?;
    assert_eq!(captured, texture_bytes);
    assert_eq!(
        texture_reader
            .read(texture_path.to_str().unwrap())
            .as_deref(),
        Some(texture_bytes.as_slice())
    );

    // A later in-place modification cannot be admitted into either context.
    fs::write(&texture, b"second-texture-revision")?;
    captured.clear();
    std::io::Cursor::new(resolver.open_asset(&resolved)?.into_buffer()?)
        .read_to_end(&mut captured)?;
    assert_eq!(captured, texture_bytes);
    assert_eq!(
        texture_reader
            .read(texture_path.to_str().unwrap())
            .as_deref(),
        Some(texture_bytes.as_slice())
    );
    let late = directory.path().join("added-after-open.bin");
    fs::write(&late, b"not-in-open-time-closure")?;
    assert!(resolver.resolve(late.to_str().unwrap()).is_none());
    assert!(texture_reader.read(late.to_str().unwrap()).is_none());

    // The same sealed resolver supports materialized history even after all
    // original source files are physically removed. No filesystem fallback.
    let root_path = fs::canonicalize(&root)?;
    let layer_path = fs::canonicalize(&layer)?;
    fs::remove_file(&root)?;
    fs::remove_file(&layer)?;
    fs::remove_file(&texture)?;
    let retained = open_retained(
        &root_path,
        [
            (root_path.clone(), root_bytes.to_vec()),
            (layer_path, layer_bytes.to_vec()),
            (texture_path, texture_bytes.to_vec()),
        ],
    )
    .context("compose removed sources from retained bytes")?;
    assert!(retained.prim(openusd::sdf::path("/World")?).is_ok());
    assert!(retained.prim(openusd::sdf::path("/Child")?).is_ok());
    Ok(())
}
