use serde::{Deserialize, Serialize};
use usd_project::ProjectId;

/// Stable, sanitized reason codes for a Project read failure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectReadErrorCode {
    ManifestUnavailable,
    RegistryIdentityMismatch,
    RegistryUnavailable,
    RepositoryUnavailable,
    InvalidProjectData,
}

/// Typed errors safe to return from the native host to the frontend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
pub enum ProjectReadError {
    #[error("unsupported Project read protocol version {actual}; expected {expected}")]
    UnsupportedProtocolVersion { expected: u16, actual: u16 },
    #[error("Project {project_id} was not found")]
    NotFound { project_id: ProjectId },
    #[error("Project {project_id} is unavailable ({code:?})")]
    Unavailable {
        project_id: ProjectId,
        code: ProjectReadErrorCode,
    },
    #[error("Project host is unavailable ({code:?})")]
    HostUnavailable { code: ProjectReadErrorCode },
    #[error("Project read request and response do not match")]
    InvalidResponse,
}
