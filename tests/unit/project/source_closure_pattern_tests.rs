use std::fs;

use anyhow::Result;
use tempfile::tempdir;

use super::{discovery, materialize_source_closure};

fn assert_localized_root_is_composed(path: &std::path::Path) -> Result<()> {
    let stage = openusd::usd::Stage::builder()
        .load(openusd::usd::InitialLoadSet::LoadNone)
        .open(path.to_str().unwrap())?;
    assert!(stage.composition_errors().is_empty());
    Ok(())
}

#[test]
fn templated_value_clips_localize_the_expanded_layer_set() -> Result<()> {
    let source_directory = tempdir()?;
    fs::write(
        source_directory.path().join("clip.1.usda"),
        "#usda 1.0\ndef Xform \"Model\" { float size = 1 }\n",
    )?;
    fs::write(
        source_directory.path().join("clip.2.usda"),
        "#usda 1.0\ndef Xform \"Model\" { float size = 2 }\n",
    )?;
    fs::write(
        source_directory.path().join("clip.9.usda"),
        "#usda 1.0\ndef Xform \"Model\" { float size = 9 }\n",
    )?;
    let source = source_directory.path().join("root.usda");
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Model\" )\ndef Xform \"Model\" ( clips = { dictionary default = { asset templateAssetPath = @./clip.#.usda@ double templateStartTime = 1 double templateEndTime = 2 double templateStride = 1 string primPath = \"/Model\" } } ) {}\n",
    )?;

    let report = discovery::discover(&source)?;
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert!(
        report
            .layers
            .iter()
            .any(|path| path.ends_with("clip.1.usda"))
    );
    assert!(
        report
            .layers
            .iter()
            .any(|path| path.ends_with("clip.2.usda"))
    );
    assert!(
        !report
            .layers
            .iter()
            .any(|path| path.ends_with("clip.9.usda"))
    );

    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("closure");
    materialize_source_closure(&source, &destination)?;
    assert!(destination.join("clip.1.usda").is_file());
    assert!(destination.join("clip.2.usda").is_file());
    assert!(!destination.join("clip.9.usda").exists());
    drop(source_directory);
    assert_localized_root_is_composed(&destination.join("root.usda"))?;
    Ok(())
}
