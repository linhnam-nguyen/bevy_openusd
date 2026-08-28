use serde::{Deserialize, Serialize};
use usd_project::{ProjectId, ProjectRoot};

/// Version of the Project-to-render-host stage activation contract.
pub const PROJECT_ACTIVATION_PROTOCOL_VERSION: u16 = 1;

/// Project/root identity requested by the active Project coordinator.
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
    pub root: ProjectRoot,
}

impl ProjectActivationCommand {
    pub fn new(
        request_id: impl Into<String>,
        generation: u64,
        project_id: ProjectId,
        root: ProjectRoot,
    ) -> Self {
        Self {
            protocol_version: PROJECT_ACTIVATION_PROTOCOL_VERSION,
            request_id: request_id.into(),
            generation,
            project_id,
            root,
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
/// requested Project root through the existing stage lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectActivationResult {
    Activated {
        generation: u64,
        project_id: ProjectId,
        root: ProjectRoot,
    },
    Failed {
        generation: u64,
        project_id: ProjectId,
        root: ProjectRoot,
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
                root: command.root.clone(),
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
                root: command.root.clone(),
                message: message.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use usd_project::SceneId;

    #[test]
    fn activation_command_and_reply_round_trip_without_a_path() {
        let command = ProjectActivationCommand::new(
            "activation-1",
            4,
            ProjectId::new_v4(),
            ProjectRoot::Scene(SceneId::new_v4()),
        );
        command.validate().unwrap();

        let encoded = serde_json::to_string(&ProjectActivationReply::activated(&command)).unwrap();
        let decoded: ProjectActivationReply = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, ProjectActivationReply::activated(&command));
        assert!(!encoded.contains("/Users/"));
        assert!(!encoded.contains("PathBuf"));
    }
}
