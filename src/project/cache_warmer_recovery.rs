//! Conservative cache recovery for authoritative Project transaction failures.

use std::path::Path;

use super::super::{
    cache_hydration::default_project_cache_config_hash, catalog::manifest_store::ManifestStore,
};
use super::{ProjectCacheTarget, ProjectCacheWarmQueue, SceneCacheStore};

/// Establish a Scene-generation boundary after a committed Project cannot be
/// classified precisely. Warm scheduling remains advisory; an old Scene cache
/// must not remain reusable merely because admission failed.
pub(crate) fn enqueue_project_targets_fail_closed(
    queue: &ProjectCacheWarmQueue,
    project_root: &Path,
) -> bool {
    if queue.enqueue_project_targets(project_root) {
        return true;
    }

    let store = SceneCacheStore::new(project_root);
    let config_hash = default_project_cache_config_hash();
    match ManifestStore::read_validated(project_root) {
        Ok(manifest) => {
            let mut all_boundaries_safe = true;
            for scene in manifest.scenes() {
                if let Err(error) = store.force_invalidate_generation(scene.id, config_hash) {
                    log::error!(
                        "failed to force-invalidate Scene cache {} after Project classification failure: {error:#}",
                        scene.id
                    );
                    let target = ProjectCacheTarget::Scene {
                        id: scene.id.to_string(),
                    };
                    if queue.remove_target_descriptors(project_root, &target) {
                        log::warn!(
                            "removed Scene cache state after force-invalidation failed for {}",
                            scene.id
                        );
                    } else {
                        log::error!(
                            "failed to remove Scene cache state after force-invalidation failed for {}",
                            scene.id
                        );
                        all_boundaries_safe = false;
                    }
                }
            }
            all_boundaries_safe
        }
        Err(error) => {
            log::error!(
                "committed Project manifest could not be read for conservative cache recovery: {error:#}"
            );
            match store.remove_all_derived_cache() {
                Ok(()) => true,
                Err(cleanup_error) => {
                    log::error!(
                        "conservative removal of derived Project cache failed: {cleanup_error:#}"
                    );
                    false
                }
            }
        }
    }
}
