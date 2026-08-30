//! Typed process-boundary handoff for Project-owned LiveStage authority.
//!
//! The Project application service owns Git and canonical files. The render
//! server owns the non-send LiveStage. This small request/response outbox lets
//! the service ask the active-stage owner for one revision-checked root layer
//! without moving OpenUSD handles across the process boundary.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use project_protocol::{ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode};
use usd_bevy::LiveRevision;
use usd_project::{ProjectId, SceneId};
use uuid::Uuid;

#[path = "runtime_authority_protocol.rs"]
mod protocol;
#[path = "runtime_authority_registry.rs"]
mod registry;
pub(crate) use protocol::{ProjectRuntimeEnvelope, ProjectRuntimeRequest, ProjectRuntimeResponse};
pub(crate) use registry::{register_project_root, registered_project_roots, unix_time_ms};

const RUNTIME_DIRECTORY: &str = ".usdhub/cache/project-runtime-authority";
const REQUESTS_DIRECTORY: &str = "requests";
const RESPONSES_DIRECTORY: &str = "responses";
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone, Debug)]
pub struct ProjectRuntimeSnapshot {
    pub lease_id: String,
    pub session_id: u64,
    pub scene_id: SceneId,
    pub live_revision: LiveRevision,
    pub root_layer: Vec<u8>,
}

pub trait ProjectRuntimeAuthority: Send + Sync {
    fn begin_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        target: &ProjectCommitTarget,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError>;

    fn finish_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        lease_id: &str,
        revision: &str,
        live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError>;

    fn validate_commit(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        lease_id: &str,
        live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError>;

    fn abort_commit(&self, project_root: &Path, project_id: ProjectId, lease_id: &str);

    fn snapshot_for_export(
        &self,
        project_root: &Path,
        project_id: ProjectId,
        scene_id: SceneId,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError>;
}

#[derive(Clone, Default)]
pub struct NoopProjectRuntimeAuthority;

impl ProjectRuntimeAuthority for NoopProjectRuntimeAuthority {
    fn begin_commit(
        &self,
        _project_root: &Path,
        _project_id: ProjectId,
        _target: &ProjectCommitTarget,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        Ok(None)
    }

    fn finish_commit(
        &self,
        _project_root: &Path,
        _project_id: ProjectId,
        _lease_id: &str,
        _revision: &str,
        _live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        Ok(())
    }

    fn validate_commit(
        &self,
        _project_root: &Path,
        _project_id: ProjectId,
        _lease_id: &str,
        _live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        Ok(())
    }

    fn abort_commit(&self, _project_root: &Path, _project_id: ProjectId, _lease_id: &str) {}

    fn snapshot_for_export(
        &self,
        _project_root: &Path,
        _project_id: ProjectId,
        _scene_id: SceneId,
    ) -> Result<Option<ProjectRuntimeSnapshot>, ProjectWriteError> {
        Ok(None)
    }
}

#[derive(Clone)]
pub struct ProjectRuntimeAuthorityQueue {
    file_lock: Arc<Mutex<()>>,
    timeout: Duration,
    request_ttl: Duration,
}

impl Default for ProjectRuntimeAuthorityQueue {
    fn default() -> Self {
        Self {
            file_lock: Arc::new(Mutex::new(())),
            timeout: RESPONSE_TIMEOUT,
            request_ttl: RESPONSE_TIMEOUT.saturating_add(Duration::from_millis(250)),
        }
    }
}

impl ProjectRuntimeAuthorityQueue {
    #[cfg(test)]
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        Self {
            file_lock: Arc::new(Mutex::new(())),
            timeout,
            request_ttl: timeout,
        }
    }

    pub(crate) fn consume_pending(
        &self,
        project_root: &Path,
    ) -> Result<Vec<ProjectRuntimeEnvelope>, ProjectWriteError> {
        let _guard = self
            .file_lock
            .lock()
            .expect("Project runtime authority queue is not poisoned");
        let directory = runtime_root(project_root).join(REQUESTS_DIRECTORY);
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(directory)
            .map_err(|_| runtime_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| runtime_error())?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut requests = Vec::new();
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|_| runtime_error())?;
            let envelope = serde_json::from_slice(&bytes).map_err(|_| runtime_error())?;
            fs::remove_file(path).map_err(|_| runtime_error())?;
            requests.push(envelope);
        }
        Ok(requests)
    }

    pub(crate) fn publish_response(
        &self,
        project_root: &Path,
        response: &ProjectRuntimeResponse,
    ) -> Result<(), ProjectWriteError> {
        let request_id = match response {
            ProjectRuntimeResponse::Ready { request_id, .. }
            | ProjectRuntimeResponse::Finished { request_id }
            | ProjectRuntimeResponse::Validated { request_id }
            | ProjectRuntimeResponse::Inactive { request_id }
            | ProjectRuntimeResponse::Failed { request_id, .. } => request_id,
        };
        let directory = runtime_root(project_root).join(RESPONSES_DIRECTORY);
        fs::create_dir_all(&directory).map_err(|_| runtime_error())?;
        let path = directory.join(format!("{request_id}.json"));
        let temporary = directory.join(format!("{request_id}.{}.tmp", Uuid::new_v4()));
        let bytes = serde_json::to_vec(response).map_err(|_| runtime_error())?;
        fs::write(&temporary, bytes).map_err(|_| runtime_error())?;
        fs::rename(temporary, path).map_err(|_| runtime_error())
    }

    fn submit_and_wait(
        &self,
        project_root: &Path,
        request: ProjectRuntimeRequest,
    ) -> Result<Option<ProjectRuntimeResponse>, ProjectWriteError> {
        let request_id = request.request_id().to_owned();
        register_project_root(request.project_id(), project_root);
        let directory = runtime_root(project_root).join(REQUESTS_DIRECTORY);
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

        let response_path = runtime_root(project_root)
            .join(RESPONSES_DIRECTORY)
            .join(format!("{request_id}.json"));
        let deadline = Instant::now() + self.timeout;
        loop {
            if response_path.is_file() {
                let bytes = fs::read(&response_path).map_err(|_| runtime_error())?;
                let response = serde_json::from_slice(&bytes).map_err(|_| runtime_error())?;
                let _ = fs::remove_file(response_path);
                return Ok(Some(response));
            }
            if Instant::now() >= deadline {
                let _guard = self
                    .file_lock
                    .lock()
                    .expect("Project runtime authority queue is not poisoned");
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

fn runtime_root(project_root: &Path) -> PathBuf {
    project_root.join(RUNTIME_DIRECTORY)
}

fn runtime_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    }
}
