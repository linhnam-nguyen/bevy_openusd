use anyhow::Result;

pub(super) fn maybe_fail_after_ordinary_scene() -> Result<()> {
    super::super::failure_injection::maybe(1)
}

pub(super) fn maybe_fail_before_manifest() -> Result<()> {
    super::super::failure_injection::maybe(2)
}

pub(super) fn maybe_fail_after_model_directory() -> Result<()> {
    super::super::failure_injection::maybe(4)
}

pub(super) fn maybe_fail_after_manifest() -> Result<()> {
    super::super::failure_injection::maybe(3)
}
