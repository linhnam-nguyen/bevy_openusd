use serde::{Deserialize, Serialize};
use usd_project::{ModelId, ProjectId, ProjectRoot, SceneId};

/// Version of the Project-to-render-host stage activation contract.
pub const PROJECT_ACTIVATION_PROTOCOL_VERSION: u16 = 3;

/// The identity a Project activation request asks the render host to load.
///
/// `ProjectRoot` means the root declared by the Project manifest. `Scene` and
/// `Model` deliberately represent standalone content targets and are not
/// Project roots, even when a root happens to resolve to the same layer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectStageTarget {
    ProjectRoot(ProjectRoot),
    Scene(SceneId),
    Model(ModelId),
}

/// Project/stage identity requested by the active Project coordinator.
///
/// The request deliberately contains no repository locator or filesystem
/// path. The render host resolves the identity through its private Project
/// application boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectActivationCommand {
    pub protocol_version: u16,
    pub request_id: String,
    pub generation: u64,
    pub project_id: ProjectId,
    pub target: ProjectStageTarget,
}

impl ProjectActivationCommand {
    pub fn new(
        request_id: impl Into<String>,
        generation: u64,
        project_id: ProjectId,
        target: ProjectStageTarget,
    ) -> Self {
        Self {
            protocol_version: PROJECT_ACTIVATION_PROTOCOL_VERSION,
            request_id: request_id.into(),
            generation,
            project_id,
            target,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectActivationError> {
        if self.protocol_version != PROJECT_ACTIVATION_PROTOCOL_VERSION {
            return Err(ProjectActivationError::UnsupportedProtocolVersion {
                expected: PROJECT_ACTIVATION_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        if self.request_id.trim().is_empty() {
            return Err(ProjectActivationError::EmptyRequestId);
        }
        if self.generation == 0 {
            return Err(ProjectActivationError::InvalidGeneration);
        }
        Ok(())
    }
}

/// Host-safe validation failures for a Project activation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum ProjectActivationError {
    #[error("unsupported Project activation protocol version {actual}; expected {expected}")]
    UnsupportedProtocolVersion { expected: u16, actual: u16 },
    #[error("Project activation request ID must not be empty")]
    EmptyRequestId,
    #[error("Project activation generation must be greater than zero")]
    InvalidGeneration,
}

/// Result emitted by the render host after it has admitted and opened the
/// requested Project stage target through the existing stage lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectActivationResult {
    Activated {
        generation: u64,
        project_id: ProjectId,
        target: ProjectStageTarget,
    },
    Failed {
        generation: u64,
        project_id: ProjectId,
        target: ProjectStageTarget,
        message: String,
    },
}

/// Response sent on the reliable application channel. It contains only
/// stable identity and a sanitized diagnostic, never the resolved path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectActivationReply {
    pub protocol_version: u16,
    pub request_id: String,
    pub result: ProjectActivationResult,
}

impl ProjectActivationReply {
    pub fn activated(command: &ProjectActivationCommand) -> Self {
        Self {
            protocol_version: PROJECT_ACTIVATION_PROTOCOL_VERSION,
            request_id: command.request_id.clone(),
            result: ProjectActivationResult::Activated {
                generation: command.generation,
                project_id: command.project_id,
                target: command.target.clone(),
            },
        }
    }

    pub fn failed(command: &ProjectActivationCommand, message: impl Into<String>) -> Self {
        Self {
            protocol_version: PROJECT_ACTIVATION_PROTOCOL_VERSION,
            request_id: command.request_id.clone(),
            result: ProjectActivationResult::Failed {
                generation: command.generation,
                project_id: command.project_id,
                target: command.target.clone(),
                message: message.into(),
            },
        }
    }

    /// Returns whether this reply still belongs to the exact activation
    /// request that is awaiting completion. Callers must not accept a reply
    /// by generation alone because another Project may reuse that number.
    pub fn matches_command(&self, command: &ProjectActivationCommand) -> bool {
        if self.protocol_version != PROJECT_ACTIVATION_PROTOCOL_VERSION
            || self.request_id != command.request_id
        {
            return false;
        }
        match &self.result {
            ProjectActivationResult::Activated {
                generation,
                project_id,
                target,
            }
            | ProjectActivationResult::Failed {
                generation,
                project_id,
                target,
                ..
            } => {
                *generation == command.generation
                    && *project_id == command.project_id
                    && target == &command.target
            }
        }
    }
}
