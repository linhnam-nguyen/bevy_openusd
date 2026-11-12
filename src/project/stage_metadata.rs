//! Stage-level metadata copied onto USDHub-owned composition wrappers.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use openusd::{
    ar::DefaultResolver,
    usd::{InitialLoadSet, Stage},
};

/// Preserve the source playback domain without copying or flattening source
/// opinions. The wrapper owns the effective rate and authored range used by
/// the viewport clock; referenced time samples remain source-owned.
pub(crate) fn copy_source_time_metadata(source: &Path, wrapper: &Stage) -> Result<()> {
    let source_string = source
        .to_str()
        .context("USD source path must be valid UTF-8")?;
    let resolver = DefaultResolver::with_search_paths([source.parent().unwrap_or(Path::new("."))]);
    let source = crate::project::source_closure::open_stage_with_resolver(
        source,
        Arc::new(resolver),
        InitialLoadSet::LoadNone,
    )
    .with_context(|| format!("open source playback metadata {source_string}"))?;
    if source.has_authored_time_code_range() {
        wrapper.set_start_time_code(source.start_time_code())?;
        wrapper.set_end_time_code(source.end_time_code())?;
    }
    wrapper.set_time_codes_per_second(source.time_codes_per_second())?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/project/stage_metadata_tests.rs"]
mod tests;
