use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};

pub(super) fn validate_localized_root(path: &Path) -> Result<()> {
    let path_string = path
        .to_str()
        .context("localized USD root path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path_string)
        .context("reopen localized USD root")?;
    stage
        .traverse(PrimPredicate::ALL, |_| {})
        .context("traverse localized USD root")?;
    ensure!(
        stage.composition_errors().is_empty(),
        "localized USD root has composition errors"
    );
    let inspection = crate::project::scene::inspection::inspect_composition(path)
        .context("reinspect localized USD root")?;
    ensure!(
        !matches!(
            inspection.classification,
            usd_project::CompositionClassification::Unsupported
        ),
        "localized USD root failed composition inspection: {:?}",
        inspection.diagnostics
    );
    Ok(())
}
