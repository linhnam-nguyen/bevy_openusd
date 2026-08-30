//! Typed process-boundary handoff for Project-owned LiveStage authority.
//!
//! The Project application service owns Git and canonical files. The render
//! server owns the non-send LiveStage. This small request/response outbox lets
//! the service ask the active-stage owner for one revision-checked root layer
//! without moving OpenUSD handles across the process boundary.

use std::path::Path;

use project_protocol::{ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode};
use usd_bevy::LiveRevision;
use usd_project::{ProjectId, SceneId};

#[path = "runtime_authority_protocol.rs"]
mod protocol;
#[path = "runtime_authority_queue.rs"]
mod queue;
#[path = "runtime_authority_registry.rs"]
mod registry;
pub(crate) use protocol::{ProjectRuntimeEnvelope, ProjectRuntimeRequest, ProjectRuntimeResponse};
pub use queue::ProjectRuntimeAuthorityQueue;
pub(crate) use registry::unix_time_ms;

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

    fn renew_commit_lease(
        &self,
        _project_root: &Path,
        _project_id: ProjectId,
        _lease_id: &str,
        _live_revision: LiveRevision,
    ) -> Result<(), ProjectWriteError> {
        Ok(())
    }

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

fn runtime_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    }
}
