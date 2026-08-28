use serde::{Deserialize, Serialize};
use usd_project::CompositionInspection;

use crate::{LocalSelectionToken, ProjectImportProgress, ProjectWriteError};

pub const PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelPreparationRequest {
    pub source: LocalSelectionToken,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelPreparationCommand {
    pub protocol_version: u16,
    pub request: ProjectModelPreparationRequest,
}

impl ProjectModelPreparationCommand {
    pub fn new(request: ProjectModelPreparationRequest) -> Self {
        Self {
            protocol_version: PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectWriteError> {
        if self.protocol_version != PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION {
            return Err(ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelPreparationResult {
    pub operation_id: String,
    pub generation: u64,
    pub progress: ProjectImportProgress,
    pub inspection: Result<CompositionInspection, ProjectWriteError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelPreparationReply {
    pub protocol_version: u16,
    pub result: Result<ProjectModelPreparationResult, ProjectWriteError>,
}

impl ProjectModelPreparationReply {
    pub fn success(result: ProjectModelPreparationResult) -> Self {
        Self {
            protocol_version: PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION,
            result: Ok(result),
        }
    }

    pub fn failure(error: ProjectWriteError) -> Self {
        Self {
            protocol_version: PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION,
            result: Err(error),
        }
    }
}
