use project_protocol::{ProjectCommitTarget, ProjectWriteErrorCode};
use serde::{Deserialize, Serialize};
use usd_project::{ProjectId, SceneId};

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
            | Self::ExportScene { request_id, .. } => request_id,
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
    Inactive {
        request_id: String,
    },
    Failed {
        request_id: String,
        code: ProjectWriteErrorCode,
    },
}
