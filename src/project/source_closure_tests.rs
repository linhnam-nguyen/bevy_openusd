use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use openusd::{
    ar::DefaultResolver,
    usd::{InitialLoadSet, Stage},
    usdz::ArchiveWriter,
};
use tempfile::tempdir;

use super::{
    discovery, materialize_source_closure, materialize_source_closure_with_resolver,
    source_closure_fingerprint,
};

fn write_composed_source(directory: &Path) -> PathBuf {
    fs::write(directory.join("texture.bin"), b"texture").unwrap();
    fs::write(directory.join("notes.txt"), b"unrelated").unwrap();
    fs::write(
        directory.join("payload.usda"),
        "#usda 1.0\n(\n defaultPrim = \"Payload\"\n)\ndef Xform \"Payload\" {}\n",
    )
    .unwrap();
    fs::write(
        directory.join("dependency.usda"),
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" {\n asset texture = @./texture.bin@\n}\n",
    )
    .unwrap();
    let source = directory.join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (\n references = @./dependency.usda@</Asset>\n payload = @./payload.usda@</Payload>\n) {}\n",
    )
    .unwrap();
    source
}

fn assert_localized_root_is_composed(path: &Path) -> Result<()> {
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_str().unwrap())?;
    assert!(stage.composition_errors().is_empty());
    Ok(())
}

#[test]
fn discovery_tracks_only_transitive_layers_and_assets() -> Result<()> {
    let source_directory = tempdir()?;
    let source = write_composed_source(source_directory.path());
    let report = discovery::discover(&source)?;

    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert!(
        report
            .layers
            .iter()
            .any(|path| path.ends_with("dependency.usda"))
    );
    assert!(
        report
            .layers
            .iter()
            .any(|path| path.ends_with("payload.usda"))
    );
    assert!(
        report
            .non_layer_assets
            .iter()
            .any(|path| path.ends_with("texture.bin"))
    );
    assert!(
        !report
            .non_layer_assets
            .iter()
            .any(|path| path.ends_with("notes.txt"))
    );
    Ok(())
}

#[test]
fn materialization_rewrites_exact_closure_and_survives_source_removal() -> Result<()> {
    let source_directory = tempdir()?;
    let source = write_composed_source(source_directory.path());
    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    let source_name = materialize_source_closure(&source, &destination)?;

    assert_eq!(source_name, "assembly.usda");
    assert!(destination.join("assembly.usda").is_file());
    assert!(destination.join("dependency.usda").is_file());
    assert!(destination.join("payload.usda").is_file());
    assert!(destination.join("texture.bin").is_file());
    assert!(!destination.join("notes.txt").exists());

    drop(source_directory);
    assert_localized_root_is_composed(&destination.join(source_name))?;
    Ok(())
}

#[test]
fn clean_import_ignores_a_large_unrelated_neighbor_set() -> Result<()> {
    let source_directory = tempdir()?;
    let source = write_composed_source(source_directory.path());
    for index in 0..100 {
        fs::write(
            source_directory
                .path()
                .join(format!("unrelated-{index}.txt")),
            format!("unrelated {index}"),
        )?;
    }
    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");

    materialize_source_closure(&source, &destination)?;

    let copied_files = fs::read_dir(&destination)?.count();
    assert_eq!(copied_files, 4, "only the exact USD closure is copied");
    assert!(!destination.join("unrelated-99.txt").exists());
    Ok(())
}

#[test]
fn canonical_project_dependency_report_rejects_external_resolution() -> Result<()> {
    let project = tempdir()?;
    let inside = project.path().join("inside.usda");
    fs::write(&inside, "#usda 1.0\ndef Xform \"Inside\" {}\n")?;
    let root = project.path().join("root.usda");
    fs::write(
        &root,
        "#usda 1.0\ndef Xform \"Root\" (references = @./inside.usda@</Inside>) {}\n",
    )?;
    let report = super::dependency_containment_report(project.path(), &root)?;
    assert!(report.unresolved.is_empty());

    let outside_directory = tempdir()?;
    let outside = outside_directory.path().join("outside.usda");
    fs::write(&outside, "#usda 1.0\ndef Xform \"Outside\" {}\n")?;
    fs::write(
        &root,
        format!(
            "#usda 1.0\ndef Xform \"Root\" (references = @{}@</Outside>) {{}}\n",
            outside.display()
        ),
    )?;
    assert!(super::dependency_containment_report(project.path(), &root).is_err());
    Ok(())
}

#[test]
fn discovery_uses_configured_resolver_search_paths_for_external_layers() -> Result<()> {
    let source_directory = tempdir()?;
    let resolver_directory = tempdir()?;
    let source = source_directory.path().join("root.usda");
    let resolved_layer = resolver_directory.path().join("configured.usda");
    fs::write(
        &resolved_layer,
        "#usda 1.0\n( defaultPrim = \"Asset\" )\ndef Xform \"Asset\" {}\n",
    )?;
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Root\" )\ndef Xform \"Root\" (references = @configured.usda@</Asset>) {}\n",
    )?;
    let resolved_layer = fs::canonicalize(resolved_layer)?;

    let resolver =
        DefaultResolver::with_search_paths([source_directory.path(), resolver_directory.path()]);
    let report = discovery::discover_with_resolver(&source, Arc::new(resolver))?;

    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert!(report.layers.iter().any(|path| path == &resolved_layer));
    Ok(())
}

#[test]
fn materialization_keeps_one_configured_resolver_through_localization_and_validation() -> Result<()>
{
    let source_directory = tempdir()?;
    let resolver_directory = tempdir()?;
    let source = source_directory.path().join("root.usda");
    let configured_layer = resolver_directory.path().join("configured.usda");
    fs::write(
        resolver_directory.path().join("texture.1001.bin"),
        b"tile-1001",
    )?;
    fs::write(
        resolver_directory.path().join("texture.1002.bin"),
        b"tile-1002",
    )?;
    fs::write(
        &configured_layer,
        "#usda 1.0\n( defaultPrim = \"Asset\" )\ndef Xform \"Asset\" { asset texture = @texture.<UDIM>.bin@ }\n",
    )?;
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Root\" )\ndef Xform \"Root\" (references = @configured.usda@</Asset>) {}\n",
    )?;

    let resolver =
        DefaultResolver::with_search_paths([source_directory.path(), resolver_directory.path()]);
    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    materialize_source_closure_with_resolver(&source, &destination, Arc::new(resolver))?;

    let external_root = destination.join("external");
    let external_directory = fs::read_dir(&external_root)?
        .next()
        .context("custom resolver dependency directory is missing")??
        .path();
    assert!(external_directory.join("configured.usda").is_file());
    assert!(external_directory.join("texture.1001.bin").is_file());
    assert!(external_directory.join("texture.1002.bin").is_file());

    drop(source_directory);
    drop(resolver_directory);
    assert_localized_root_is_composed(&destination.join("root.usda"))?;
    Ok(())
}

#[test]
fn external_dependency_is_localized_without_copying_its_neighbor_directory() -> Result<()> {
    let source_directory = tempdir()?;
    let external_directory = tempdir()?;
    fs::write(
        external_directory.path().join("dependency.usda"),
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" {}\n",
    )?;
    fs::write(external_directory.path().join("backup.usda"), b"unrelated")?;
    let source = source_directory.path().join("assembly.usda");
    fs::write(
        &source,
        format!(
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (references = @{}@</Asset>) {{}}\n",
            external_directory.path().join("dependency.usda").display()
        ),
    )?;
    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    materialize_source_closure(&source, &destination)?;

    assert!(destination.join("external").exists());
    assert!(!destination.join("backup.usda").exists());
    assert_localized_root_is_composed(&destination.join("assembly.usda"))?;
    Ok(())
}

#[test]
fn unresolved_required_dependency_is_rejected() -> Result<()> {
    let directory = tempdir()?;
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\ndef Xform \"Assembly\" (references = @./missing.usda@</Asset>) {}\n",
    )?;
    let destination = tempdir()?.path().join("closure");
    assert!(materialize_source_closure(&source, &destination).is_err());
    Ok(())
}

#[test]
fn fingerprint_ignores_unrelated_neighbors_but_tracks_dependencies() -> Result<()> {
    let source_directory = tempdir()?;
    let source = write_composed_source(source_directory.path());
    let before = source_closure_fingerprint(&source)?;
    fs::write(
        source_directory.path().join("notes.txt"),
        b"changed unrelated",
    )?;
    assert_eq!(before, source_closure_fingerprint(&source)?);
    fs::write(
        source_directory.path().join("texture.bin"),
        b"changed dependency",
    )?;
    assert_ne!(before, source_closure_fingerprint(&source)?);
    Ok(())
}

#[test]
fn symlinked_source_is_rejected() -> Result<()> {
    let directory = tempdir()?;
    let actual = directory.path().join("actual.usda");
    fs::write(&actual, "#usda 1.0\n")?;
    let source = directory.path().join("source.usda");
    symlink(&actual, &source)?;
    let destination = tempdir()?.path().join("closure");

    assert!(materialize_source_closure(&source, &destination).is_err());
    Ok(())
}

#[test]
fn existing_destination_is_a_collision() -> Result<()> {
    let directory = tempdir()?;
    let source = directory.path().join("source.usda");
    fs::write(&source, "#usda 1.0\n")?;
    let destination = directory.path().join("closure");
    fs::create_dir_all(&destination)?;
    fs::write(destination.join("existing"), b"collision")?;

    assert!(materialize_source_closure(&source, &destination).is_err());
    Ok(())
}

#[test]
fn usd_z_package_is_preserved_as_one_exact_closure() -> Result<()> {
    let source_directory = tempdir()?;
    let source = source_directory.path().join("package.usdz");
    let mut archive = ArchiveWriter::create(&source)?;
    archive.add_layer(
        "root.usda",
        b"#usda 1.0\n(defaultPrim = \"Root\")\ndef Xform \"Root\" (references = @./inner.usda@</Inner>) {}\n",
    )?;
    archive.add_layer("inner.usda", b"#usda 1.0\ndef Xform \"Inner\" {}\n")?;
    archive.finish()?;

    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    let source_name = materialize_source_closure(&source, &destination)?;

    assert_eq!(source_name, "package.usdz");
    assert!(destination.join("package.usdz").is_file());
    assert_localized_root_is_composed(&destination.join(source_name))?;
    Ok(())
}

#[test]
fn nested_package_identifier_is_localized_as_an_atomic_archive() -> Result<()> {
    let source_directory = tempdir()?;
    let package = source_directory.path().join("package.usdz");
    let mut archive = ArchiveWriter::create(&package)?;
    archive.add_layer("inner.usda", b"#usda 1.0\ndef Xform \"Inner\" {}\n")?;
    archive.finish()?;

    let source = source_directory.path().join("root.usda");
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Root\" )\ndef Xform \"Root\" (references = @./package.usdz[inner.usda]@</Inner>) {}\n",
    )?;
    let destination = tempdir()?.path().join("closure");

    materialize_source_closure(&source, &destination)?;

    assert!(destination.join("root.usda").is_file());
    assert!(destination.join("package.usdz").is_file());
    drop(source_directory);
    assert_localized_root_is_composed(&destination.join("root.usda"))?;
    Ok(())
}

#[test]
fn udim_assets_localize_every_matching_tile_without_copying_neighbors() -> Result<()> {
    let source_directory = tempdir()?;
    let textures = source_directory.path().join("textures");
    fs::create_dir(&textures)?;
    fs::write(textures.join("diffuse.1001.bin"), b"tile-1001")?;
    fs::write(textures.join("diffuse.1002.bin"), b"tile-1002")?;
    fs::write(textures.join("diffuse.2001.bin"), b"outside-udim-range")?;
    fs::write(textures.join("other.1001.bin"), b"unrelated-pattern")?;
    let source = source_directory.path().join("root.usda");
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Root\" upAxis = \"Y\" metersPerUnit = 1.0 )\ndef Xform \"Root\" { asset texture = @./textures/diffuse.<UDIM>.bin@ }\n",
    )?;

    let report = discovery::discover(&source)?;
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(
        report
            .non_layer_assets
            .iter()
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bin"))
            .count(),
        2
    );

    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    materialize_source_closure(&source, &destination)?;
    assert!(destination.join("textures/diffuse.1001.bin").is_file());
    assert!(destination.join("textures/diffuse.1002.bin").is_file());
    assert!(!destination.join("textures/diffuse.2001.bin").exists());
    assert!(!destination.join("textures/other.1001.bin").exists());
    Ok(())
}
