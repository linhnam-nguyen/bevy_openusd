use serde::{Deserialize, Serialize};

use crate::ProjectWriteError;

pub const PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION: u16 = 1;

/// Coarse status for one replaceable Project import operation.
///
/// Progress describes operation state, not individual files or USD prims.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectImportPhase {
    Queued,
    Inspecting,
    Preparing,
    Validating,
    Publishing,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportProgress {
    pub operation_id: String,
    pub generation: u64,
    pub phase: ProjectImportPhase,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportProgressRequest {
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportProgressCommand {
    pub protocol_version: u16,
    pub request: ProjectImportProgressRequest,
}

impl ProjectImportProgressCommand {
    pub fn new(request: ProjectImportProgressRequest) -> Self {
        Self {
            protocol_version: PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectWriteError> {
        if self.protocol_version != PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION {
            return Err(ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportProgressReply {
    pub protocol_version: u16,
    pub result: Result<Option<ProjectImportProgress>, ProjectWriteError>,
}

impl ProjectImportProgressReply {
    pub fn success(progress: Option<ProjectImportProgress>) -> Self {
        Self {
            protocol_version: PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION,
            result: Ok(progress),
        }
    }

    pub fn failure(error: ProjectWriteError) -> Self {
        Self {
            protocol_version: PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION,
            result: Err(error),
        }
    }
}
