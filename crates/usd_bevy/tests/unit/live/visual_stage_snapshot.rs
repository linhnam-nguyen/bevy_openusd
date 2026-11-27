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
        bindings: visual.bindings.clone(),
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

#[test]
fn frozen_binding_table_rejects_conflicting_resolution() -> Result<()> {
    let mut bindings = SourceBindings::default();
    assert!(bindings.observe("alias.bin", Path::new("first.bin")));
    let first = bindings.identity_bytes()?;
    assert_eq!(bindings.resolve(Path::new("alias.bin")), Some(&PathBuf::from("first.bin")));
    assert!(!bindings.observe("alias.bin", Path::new("second.bin")));
    assert!(bindings.identity_bytes().is_err());
    assert_eq!(bindings.resolve(Path::new("alias.bin")), Some(&PathBuf::from("first.bin")));
    let mut changed = SourceBindings::default();
    assert!(changed.observe("alias.bin", Path::new("second.bin")));
    assert_ne!(first, changed.identity_bytes()?);
    Ok(())
}

#[test]
fn frozen_component_bindings_resolve_mixed_short_and_long_spellings() {
    let mut bindings = SourceBindings::default();
    let full = PathBuf::from("LongParent").join("LongChild").join("texture.bin");
    assert!(bindings.insert_file(&full, &full));
    bindings.short_prefixes.insert(PathBuf::from("LONGPA~1"), PathBuf::from("LongParent"));
    bindings.short_prefixes.insert(
        PathBuf::from("LongParent").join("LONGCH~1"),
        PathBuf::from("LongParent").join("LongChild"),
    );
    for spelling in [
        PathBuf::from("LONGPA~1").join("LongChild").join("texture.bin"),
        PathBuf::from("LongParent").join("LONGCH~1").join("texture.bin"),
        PathBuf::from("LONGPA~1").join("LONGCH~1").join("texture.bin"),
    ] {
        assert_eq!(bindings.resolve(&spelling), Some(&full));
    }
    assert!(bindings.resolve(Path::new("unknown/../LongParent/LongChild/texture.bin")).is_none());
}

#[cfg(any(unix, windows))]
fn create_asset_directory_alias(target: &Path, alias: &Path) -> Result<()> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, alias)?;
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"])
        .arg(alias).arg(target).status()?.success(), "create test directory junction");
    Ok(())
}

#[cfg(any(unix, windows))]
fn remove_asset_directory_alias(alias: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::remove_file(alias)?;
    #[cfg(windows)]
    fs::remove_dir(alias)?;
    Ok(())
}

#[cfg(any(unix, windows))]
#[test]
fn sealed_texture_alias_cannot_select_another_captured_resource_after_retargeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root_dir = directory.path();
    let root = root_dir.join("root.usda");
    let a = root_dir.join("source_a");
    let b = root_dir.join("source_b");
    let alias = root_dir.join("asset_alias");
    fs::create_dir(&a)?;
    fs::create_dir(&b)?;
    fs::write(a.join("texture.bin"), b"captured A")?;
    fs::write(b.join("texture.bin"), b"captured B")?;
    fs::write(&root, br#"#usda 1.0
(
    defaultPrim = "World"
)
def Xform "World" {
    asset userProperties:alias = @./asset_alias/texture.bin@
    asset userProperties:first = @./source_a/texture.bin@
    asset userProperties:second = @./source_b/texture.bin@
}
"#)?;
    create_asset_directory_alias(&a, &alias)?;
    let (_, original) = VisualStage::open_pair(&root)?;
    let reader = original.texture_source((1, 0));
    let alias_texture = fs::canonicalize(root_dir)?.join("asset_alias").join("texture.bin");
    let spelling = alias_texture.to_str().context("test alias UTF-8")?;
    let first = original.opened_resolver_bindings()?;
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured A".as_slice()));
    assert!(original.source.borrow().contains_key(&fs::canonicalize(b.join("texture.bin"))?));

    // The old resolver must continue selecting A even though B is both a
    // captured source and the alias's current physical destination.
    remove_asset_directory_alias(&alias)?;
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured A".as_slice()));
    create_asset_directory_alias(&b, &alias)?;
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured A".as_slice()));
    let (_, changed) = VisualStage::open_pair(&root)?;
    assert_eq!(changed.texture_source((2, 0)).read(spelling).as_deref(), Some(b"captured B".as_slice()));
    assert_ne!(first, changed.opened_resolver_bindings()?);
    remove_asset_directory_alias(&alias)?;
    create_asset_directory_alias(&a, &alias)?;
    let (_, restored) = VisualStage::open_pair(&root)?;
    assert_eq!(first, restored.opened_resolver_bindings()?);
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured A".as_slice()));
    remove_asset_directory_alias(&alias)?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn real_windows_short_name_lookup_survives_source_removal() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("long_document_name.usda");
    let texture = directory.path().join("long_texture_name.bin");
    fs::write(&root, b"#usda 1.0\ndef Xform \"World\" {\n asset userProperties:albedo = @./long_texture_name.bin@\n}\n")?;
    fs::write(&texture, b"captured short name texture")?;
    let (_, visual) = VisualStage::open_pair(&root)?;
    let canonical = fs::canonicalize(&texture)?;
    let plain = binding_key(&canonical);
    let short = visual.bindings.borrow().short_prefixes.iter().find_map(|(alias, prefix)| {
        plain.strip_prefix(prefix).ok().map(|suffix| {
            if suffix.as_os_str().is_empty() { alias.clone() } else { alias.join(suffix) }
        })
    });
    let Some(short) = short else {
        // 8.3 names may be disabled on this volume; the pure component test
        // still checks the invariant. The supported NTFS fixture must be run
        // on a volume with short names to qualify the OS-specific branch.
        eprintln!("8.3 short names unavailable on test volume");
        return Ok(());
    };
    assert_eq!(fs::canonicalize(&short)?, canonical);
    let spelling = short.to_str().context("test short name UTF-8")?;
    let reader = visual.texture_source((1, 0));
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured short name texture".as_slice()));
    fs::remove_file(&texture)?;
    assert_eq!(reader.read(spelling).as_deref(), Some(b"captured short name texture".as_slice()));
    Ok(())
}
