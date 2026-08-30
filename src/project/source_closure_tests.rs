use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use anyhow::Result;
use openusd::usd::{InitialLoadSet, Stage};
use tempfile::tempdir;

use super::{discovery, materialize_source_closure, source_closure_fingerprint};

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
