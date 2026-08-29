use serde::{Deserialize, Serialize};
use usd_project::CompositionInspection;

use crate::{LocalSelectionToken, ProjectImportProgress, ProjectWriteError};

/// Version of the bounded composed-Scene inspection command boundary.
pub const PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION: u16 = 1;

/// An opaque frontend request for a host-resolved USD source inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneInspectionRequest {
    pub source: LocalSelectionToken,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneInspectionCommand {
    pub protocol_version: u16,
    pub request: ProjectSceneInspectionRequest,
}

impl ProjectSceneInspectionCommand {
    pub fn new(request: ProjectSceneInspectionRequest) -> Self {
        Self {
            protocol_version: PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectWriteError> {
        if self.protocol_version != PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION {
            return Err(ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        Ok(())
    }
}

/// The operation identity is returned with both success and typed failure so
/// the UI can discard stale or superseded results without guessing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectSceneInspectionResult {
    pub operation_id: String,
    pub generation: u64,
    pub progress: ProjectImportProgress,
    pub inspection: Result<CompositionInspection, ProjectWriteError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectSceneInspectionReply {
    pub protocol_version: u16,
    pub result: Result<ProjectSceneInspectionResult, ProjectWriteError>,
}

impl ProjectSceneInspectionReply {
    pub fn success(result: ProjectSceneInspectionResult) -> Self {
        Self {
            protocol_version: PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION,
            result: Ok(result),
        }
    }

    pub fn failure(error: ProjectWriteError) -> Self {
        Self {
            protocol_version: PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION,
            result: Err(error),
        }
    }
}
