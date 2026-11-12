use std::{fs, path::Path};

use anyhow::Result;
use openusd::usd::{InitialLoadSet, Stage};
use tempfile::tempdir;

use super::{discovery, materialize_source_closure};

#[test]
fn missing_renderer_assets_are_optional_but_the_localized_stage_stays_valid() -> Result<()> {
    let source_directory = tempdir()?;
    let source = source_directory.path().join("Projet1.usda");
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Assembly\" )\ndef Xform \"Assembly\" {\n    asset material = @./missing/OmniPBR.mdl@\n    asset texture = @./missing/albedo.png@\n}\n",
    )?;

    let report = discovery::discover(&source)?;
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert!(
        report
            .optional_unresolved
            .iter()
            .any(|asset| asset.contains("OmniPBR.mdl"))
    );
    assert!(
        report
            .optional_unresolved
            .iter()
            .any(|asset| asset.contains("albedo.png"))
    );

    let destination_parent = tempdir()?;
    let destination = destination_parent.path().join("localized");
    let root_name = materialize_source_closure(&source, &destination)?;
    let localized_root = destination.join(root_name);
    assert!(localized_root.is_file());
    assert_localized_stage_is_composed(&localized_root)?;
    assert!(!destination.join("missing").exists());
    Ok(())
}

fn assert_localized_stage_is_composed(path: &Path) -> Result<()> {
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_str().expect("localized path is UTF-8"))?;
    assert!(stage.composition_errors().is_empty());
    Ok(())
}
