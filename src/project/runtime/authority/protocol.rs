use project_protocol::{ProjectCommitTarget, ProjectWriteErrorCode};
use serde::{Deserialize, Serialize};
use usd_project::{ProjectId, SceneId};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ProjectRuntimeEnvelope {
    request: ProjectRuntimeRequest,
    expires_at_ms: u128,
}

impl ProjectRuntimeEnvelope {
    pub(crate) fn new(request: ProjectRuntimeRequest, expires_at_ms: u128) -> Self {
        Self {
            request,
            expires_at_ms,
        }
    }

    pub(crate) fn is_expired(&self, now_ms: u128) -> bool {
        now_ms >= self.expires_at_ms
    }

    pub(crate) fn into_request(self) -> ProjectRuntimeRequest {
        self.request
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum ProjectRuntimeRequest {
    BeginCommit {
        request_id: String,
        project_id: ProjectId,
        target: ProjectCommitTarget,
    },
    FinishCommit {
        request_id: String,
        project_id: ProjectId,
        lease_id: String,
        revision: String,
        live_revision: u64,
    },
    ValidateCommit {
        request_id: String,
        project_id: ProjectId,
        lease_id: String,
        live_revision: u64,
    },
    AbortCommit {
        request_id: String,
        project_id: ProjectId,
        lease_id: String,
    },
    RenewCommitLease {
        request_id: String,
        project_id: ProjectId,
        lease_id: String,
        live_revision: u64,
    },
    ExportScene {
        request_id: String,
        project_id: ProjectId,
        scene_id: SceneId,
    },
}

impl ProjectRuntimeRequest {
    pub(crate) fn request_id(&self) -> &str {
        match self {
            Self::BeginCommit { request_id, .. }
            | Self::FinishCommit { request_id, .. }
            | Self::ValidateCommit { request_id, .. }
            | Self::AbortCommit { request_id, .. }
            | Self::RenewCommitLease { request_id, .. }
            | Self::ExportScene { request_id, .. } => request_id,
        }
    }

    pub(crate) fn project_id(&self) -> ProjectId {
        match self {
            Self::BeginCommit { project_id, .. }
            | Self::FinishCommit { project_id, .. }
            | Self::ValidateCommit { project_id, .. }
            | Self::AbortCommit { project_id, .. }
            | Self::RenewCommitLease { project_id, .. }
            | Self::ExportScene { project_id, .. } => *project_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) enum ProjectRuntimeResponse {
    Ready {
        request_id: String,
        lease_id: String,
        session_id: u64,
        scene_id: SceneId,
        live_revision: u64,
        root_layer: Vec<u8>,
    },
    Finished {
        request_id: String,
    },
    Validated {
        request_id: String,
    },
    Renewed {
        request_id: String,
    },
    Inactive {
        request_id: String,
    },
    Failed {
        request_id: String,
        code: ProjectWriteErrorCode,
    },
}
