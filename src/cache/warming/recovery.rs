//! Conservative cache recovery for authoritative Project transaction failures.

use std::path::Path;

use super::ProjectCacheWarmQueue;
use super::super::sync;

/// Reconcile every managed target or retain the Project-wide freshness barrier.
/// Queue admission is advisory; never retire live targets to hide a failure.
pub(crate) fn enqueue_project_targets_fail_closed(
    queue: &ProjectCacheWarmQueue,
    project_root: &Path,
) -> bool {
    match queue.enqueue_project_targets_for_mutation(project_root) {
        Ok(_) => true, // durable transitions or a preserved barrier
        Err(error) => {
            log::warn!("Project cache reconciliation failed: {error:#}");
            match sync::set_cache_invalidation_barrier(project_root) {
                Ok(()) => true,
                Err(error) => {
                    log::error!("could not preserve Project cache invalidation barrier: {error:#}");
                    false
                }
            }
        }
    }
}
