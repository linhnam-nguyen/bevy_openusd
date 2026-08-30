use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use project_protocol::{ProjectCommitTarget, ProjectWriteError};
use usd_bevy::LiveRevision;
use usd_project::{ProjectId, SceneId};
use uuid::Uuid;

use super::{
    ProjectRuntimeAuthority, ProjectRuntimeEnvelope, ProjectRuntimeRequest, ProjectRuntimeResponse,
    ProjectRuntimeSnapshot, runtime_error, unix_time_ms,
};

#[path = "runtime_authority_queue_storage.rs"]
mod storage;
use storage::{REQUESTS_DIRECTORY, RESPONSES_DIRECTORY, RegistryCache};

const RESPONSE_TIMEOUT: Duration = Duration::from_millis(750);
const CLAIM_RESPONSE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct ProjectRuntimeAuthorityQueue {
    file_lock: Arc<Mutex<()>>,
    registry_cache: Arc<Mutex<RegistryCache>>,
    last_cleanup: Arc<Mutex<Option<Instant>>>,
    timeout: Duration,
    request_ttl: Duration,
    workspace_registry_path: Option<PathBuf>,
}

impl Default for ProjectRuntimeAuthorityQueue {
    fn default() -> Self {
        Self {
            file_lock: Arc::new(Mutex::new(())),
            registry_cache: Arc::new(Mutex::new(RegistryCache::default())),
            last_cleanup: Arc::new(Mutex::new(None)),
            timeout: RESPONSE_TIMEOUT,
            request_ttl: RESPONSE_TIMEOUT.saturating_add(Duration::from_millis(250)),
            workspace_registry_path: None,
        }
    }
}

impl ProjectRuntimeAuthorityQueue {
    pub fn with_workspace_registry_path(registry_path: impl Into<PathBuf>) -> Self {
        Self {
            workspace_registry_path: Some(registry_path.into()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            request_ttl: timeout,
            ..Self::default()
        }
    }

    #[cfg(test)]
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

    fn runtime_root(&self, project_root: &Path) -> PathBuf {
        storage::runtime_root(self, project_root)
    }

    fn maybe_cleanup(&self, root: &Path) {
        storage::maybe_cleanup(self, root);
    }

    pub(crate) fn consume_pending(&self) -> Result<Vec<ProjectRuntimeEnvelope>, ProjectWriteError> {
        storage::consume_pending(self)
    }

    pub(crate) fn registered_project_roots(&self) -> Vec<(ProjectId, PathBuf)> {
        storage::registered_project_roots(self)
    }

    pub(crate) fn is_cancelled(&self, project_root: &Path, request_id: &str) -> bool {
        storage::is_cancelled(self, project_root, request_id)
    }

    fn create_waiting_claim(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<(), ProjectWriteError> {
        storage::create_waiting_claim(self, project_root, request_id)
    }

    pub(crate) fn claim_request(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<bool, ProjectWriteError> {
        storage::claim_request(self, project_root, request_id)
    }

    fn cancel_waiting_request(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<bool, ProjectWriteError> {
        storage::cancel_waiting_request(self, project_root, request_id)
    }

    pub(crate) fn clear_request_state(&self, project_root: &Path, request_id: &str) {
        storage::clear_request_state(self, project_root, request_id);
    }

    #[cfg(test)]
    pub(crate) fn prepare_request_claim(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<(), ProjectWriteError> {
        self.create_waiting_claim(project_root, request_id)
    }

    #[cfg(test)]
    pub(crate) fn cancel_request_for_test(
        &self,
        project_root: &Path,
        request_id: &str,
    ) -> Result<bool, ProjectWriteError> {
        self.cancel_waiting_request(project_root, request_id)
    }

    #[cfg(test)]
    pub(crate) fn cleanup_for_test(&self, project_root: &Path) {
        storage::cleanup_for_test(self, project_root);
    }

    pub(crate) fn publish_response(
        &self,
        response: &ProjectRuntimeResponse,
    ) -> Result<(), ProjectWriteError> {
        storage::publish_response(self, response)
    }

    fn submit_and_wait(
        &self,
        project_root: &Path,
        request: ProjectRuntimeRequest,
    ) -> Result<Option<ProjectRuntimeResponse>, ProjectWriteError> {
        let request_id = request.request_id().to_owned();
        let root = self.runtime_root(project_root);
        self.maybe_cleanup(&root);
        self.create_waiting_claim(project_root, &request_id)?;
        let directory = root.join(REQUESTS_DIRECTORY);
        fs::create_dir_all(&directory).map_err(|_| runtime_error())?;
        let path = directory.join(format!("{request_id}.json"));
        let temporary = directory.join(format!("{request_id}.{}.tmp", Uuid::new_v4()));
        let envelope = ProjectRuntimeEnvelope::new(
            request,
            unix_time_ms().saturating_add(self.request_ttl.as_millis()),
        );
        let bytes = serde_json::to_vec(&envelope).map_err(|_| runtime_error())?;
        fs::write(&temporary, bytes).map_err(|_| runtime_error())?;
        fs::rename(temporary, &path).map_err(|_| runtime_error())?;

        let response_path = root
            .join(RESPONSES_DIRECTORY)
            .join(format!("{request_id}.json"));
        let deadline = Instant::now() + self.timeout;
        loop {
            if response_path.is_file() {
                let bytes = fs::read(&response_path).map_err(|_| runtime_error())?;
                let response = serde_json::from_slice(&bytes).map_err(|_| runtime_error())?;
                let _ = fs::remove_file(response_path);
                self.clear_request_state(project_root, &request_id);
                return Ok(Some(response));
            }
            if Instant::now() >= deadline {
                if response_path.is_file() {
                    continue;
                }
                if self.cancel_waiting_request(project_root, &request_id)? {
                    let _ = fs::remove_file(&path);
                    return Err(runtime_error());
                }
                let grace_deadline = Instant::now() + CLAIM_RESPONSE_GRACE;
                while Instant::now() < grace_deadline {
                    if response_path.is_file() {
                        let bytes = fs::read(&response_path).map_err(|_| runtime_error())?;
                        let response =
                            serde_json::from_slice(&bytes).map_err(|_| runtime_error())?;
                        let _ = fs::remove_file(&response_path);
                        self.clear_request_state(project_root, &request_id);
                        return Ok(Some(response));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = fs::remove_file(&path);
                return Err(runtime_error());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl ProjectRuntimeAuthority for ProjectRuntimeAuthorityQueue {
    fn begin_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        target: &ProjectCommitTarget,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        let request_id = Uuid::new_v4().to_string();
        match self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::BeginCommit {
                request_id,
                project_id,
                target: target.clone(),
            },
        )? {
            Some(ProjectRuntimeResponse::Ready {
                lease_id,
                session_id,
                scene_id,
                live_revision,
                root_layer,
                ..
            }) => Ok(Some(ProjectRuntimeSnapshot {
                lease_id,
                session_id,
                scene_id,
                live_revision: LiveRevision(live_revision),
                root_layer,
            })),
            Some(ProjectRuntimeResponse::Inactive { .. }) => Ok(None),
            Some(ProjectRuntimeResponse::Failed { code, .. }) => {
                Err(ProjectWriteError::Failed { code })
            }
            Some(_) => Err(runtime_error()),
            None => Err(runtime_error()),
        }
    }

    fn finish_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        lease_id: &str,
        revision: &str,
        live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        let request_id = Uuid::new_v4().to_string();
        let response = self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::FinishCommit {
                request_id,
                project_id,
                lease_id: lease_id.to_owned(),
                revision: revision.to_owned(),
                live_revision: live_revision.0,
            },
        )?;
        match response {
            Some(ProjectRuntimeResponse::Finished { .. }) => Ok(()),
            Some(ProjectRuntimeResponse::Failed { code, .. }) => {
                Err(ProjectWriteError::Failed { code })
            }
            Some(_) => Err(runtime_error()),
            None => Err(runtime_error()),
        }
    }

    fn validate_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        lease_id: &str,
        live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        let request_id = Uuid::new_v4().to_string();
        let response = self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::ValidateCommit {
                request_id,
                project_id,
                lease_id: lease_id.to_owned(),
                live_revision: live_revision.0,
            },
        )?;
        match response {
            Some(ProjectRuntimeResponse::Validated { .. }) => Ok(()),
            Some(ProjectRuntimeResponse::Failed { code, .. }) => {
                Err(ProjectWriteError::Failed { code })
            }
            Some(_) => Err(runtime_error()),
            None => Err(runtime_error()),
        }
    }

    fn abort_commit(&self, project_root: &Path, project_id: ProjectId, lease_id: &str) {
        let request_id = Uuid::new_v4().to_string();
        let _ = self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::AbortCommit {
                request_id,
                project_id,
                lease_id: lease_id.to_owned(),
            },
        );
    }

    fn renew_commit_lease(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        lease_id: &str,
        live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        let request_id = Uuid::new_v4().to_string();
        let response = self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::RenewCommitLease {
                request_id,
                project_id,
                lease_id: lease_id.to_owned(),
                live_revision: live_revision.0,
            },
        )?;
        match response {
            Some(ProjectRuntimeResponse::Renewed { .. }) => Ok(()),
            Some(ProjectRuntimeResponse::Failed { code, .. }) => {
                Err(ProjectWriteError::Failed { code })
            }
            Some(_) => Err(runtime_error()),
            None => Err(runtime_error()),
        }
    }

    fn snapshot_for_export(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        scene_id: SceneId,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        let request_id = Uuid::new_v4().to_string();
        match self.submit_and_wait(
            project_root,
            ProjectRuntimeRequest::ExportScene {
                request_id,
                project_id,
                scene_id,
            },
        )? {
            Some(ProjectRuntimeResponse::Ready {
                lease_id,
                session_id,
                scene_id,
                live_revision,
                root_layer,
                ..
            }) => Ok(Some(ProjectRuntimeSnapshot {
                lease_id,
                session_id,
                scene_id,
                live_revision: LiveRevision(live_revision),
                root_layer,
            })),
            Some(ProjectRuntimeResponse::Inactive { .. }) => Ok(None),
            Some(ProjectRuntimeResponse::Failed { code, .. }) => {
                Err(ProjectWriteError::Failed { code })
            }
            Some(_) => Err(runtime_error()),
            None => Err(runtime_error()),
        }
    }
}
