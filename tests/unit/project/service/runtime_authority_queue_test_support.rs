use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use project_protocol::ProjectWriteError;

use super::storage::RegistryCache;
use super::*;

impl ProjectRuntimeAuthorityQueue {
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            request_ttl: timeout,
            ..Self::default()
        }
    }

    pub(crate) fn with_timeout_and_registry_path(
        timeout: Duration,
        registry_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            file_lock: Arc::new(Mutex::new(())),
            registry_cache: Arc::new(Mutex::new(RegistryCache::default())),
            last_cleanup: Arc::new(Mutex::new(None)),
            timeout,
            request_ttl: timeout.saturating_add(Duration::from_millis(250)),
            workspace_registry_path: Some(registry_path.into()),
        }
    }

    pub(crate) fn registry_resolution_count(&self) -> usize {
        self.registry_cache
            .lock()
            .expect("Project runtime registry cache is not poisoned")
            .resolution_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn prepare_request_claim(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<(), ProjectWriteError> {
        self.create_waiting_claim(project_root, request_id)
    }

    pub(crate) fn cancel_request_for_test(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<bool, ProjectWriteError> {
        self.cancel_waiting_request(project_root, request_id)
    }

    pub(crate) fn cleanup_for_test(&self, project_root: &Path) {
        storage::cleanup_for_test(self, project_root);
    }
}
