//! Exact dependency closure handling for Project Scene and Model imports.

use std::path::Path;

use anyhow::{Context, Result, ensure};

#[path = "source_closure_discovery.rs"]
mod discovery;
#[path = "source_closure_io.rs"]
mod io;
#[path = "source_closure_localize.rs"]
mod localize;

pub(crate) use discovery::{LocalizedDependencyReport, discover};
pub(crate) use localize::{
    materialize_source_closure, materialize_source_closure_with_resolver,
    source_closure_fingerprint,
};

/// Discover a canonical Project asset and prove its complete dependency
/// closure remains inside the Project root. The discovery itself walks
/// parsed OpenUSD layer fields, references, payloads, and asset values; this
/// boundary adds the Storage v2 containment invariant.
pub(crate) fn dependency_containment_report(
    project_root: &Path,
    root_asset: &Path,
) -> Result<LocalizedDependencyReport> {
    let project_root = std::fs::canonicalize(project_root)
        .with_context(|| format!("canonicalize Project root {}", project_root.display()))?;
    let report = discover(root_asset)?;
    ensure!(
        report.unresolved.is_empty(),
        "canonical Project dependency closure has unresolved assets: {:?}",
        report.unresolved
    );
    for dependency in report
        .layers
        .iter()
        .chain(report.non_layer_assets.iter())
        .chain(std::iter::once(&report.root_asset))
    {
        ensure!(
            dependency.starts_with(&project_root),
            "canonical Project dependency escapes the Project root: {}",
            dependency.display()
        );
    }
    Ok(report)
}

#[cfg(test)]
#[path = "source_closure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "source_closure_pattern_tests.rs"]
mod pattern_tests;
