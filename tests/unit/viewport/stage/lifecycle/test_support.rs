//! Test-only helpers attached to the owning production module.
use super::*;

pub(super) fn activate_stage_with_cache_context_for_test<F>(
    world: &mut World,
    path: std::path::PathBuf,
    cache_context: Option<ActiveProjectCacheContext>,
    before_open: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    activate_stage_with_cache_context_inner(
        world,
        path,
        cache_context,
        0,
        StagePresentationContext::default(),
        before_open,
    )
}
