use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Result;
use viewport_protocol::RuntimeProfile;

use super::*;

impl LatestWarmState {
    pub(crate) fn retained_counts(&self) -> (usize, usize) {
        let inner = self
            .inner
            .lock()
            .expect("Project cache warm state is not poisoned");
        (inner.latest.len(), inner.dirty_projects.len())
    }
}

pub(crate) fn wait_for(
    _queue: &ProjectCacheWarmQueue,
    project_root: &Path,
    target: &super::super::ProjectCacheTarget,
) -> Result<Option<super::super::ProjectCacheDescriptor>> {
    let identity = super::super::ProjectCacheIdentity::for_project(
        project_root,
        target.clone(),
        RuntimeProfile::NativeMedium,
        super::super::super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    let store = super::super::ProjectCacheStore::new(project_root);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(descriptor) = store.load(&identity)? {
            if descriptor.state != super::super::ProjectCacheState::Building {
                return Ok(Some(descriptor));
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(CACHE_PREPARATION_POLL);
    }
}
