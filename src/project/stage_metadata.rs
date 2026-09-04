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
mod tests {
    use super::*;

    #[test]
    fn wrapper_preserves_authored_playback_domain() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source_path = directory.path().join("animated.usda");
        std::fs::write(
            &source_path,
            "#usda 1.0\n( startTimeCode = 1 endTimeCode = 48 timeCodesPerSecond = 24 )\ndef Xform \"Root\" {}\n",
        )?;
        let wrapper = Stage::builder().in_memory("wrapper.usda")?;

        copy_source_time_metadata(&source_path, &wrapper)?;

        assert!(wrapper.has_authored_time_code_range());
        assert_eq!(wrapper.start_time_code(), 1.0);
        assert_eq!(wrapper.end_time_code(), 48.0);
        assert_eq!(wrapper.time_codes_per_second(), 24.0);
        Ok(())
    }
}
