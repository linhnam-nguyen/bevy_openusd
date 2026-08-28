use serde::{Deserialize, Serialize};
use usd_project::{
    CompositionInspection, ModelId, ProjectId, ProjectSummary, SceneId, SceneMemberId,
};

use crate::{LocalSelectionToken, ProjectReadError};

/// Version of the shared Project write command boundary.
pub const PROJECT_WRITE_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectInspectionClassification {
    NativeUsdHub,
    AdoptableGit,
    Incompatible,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectInspectionWarning {
    BroadUsdHubIgnore,
    MissingLocalCacheRoots,
    TrackedDerivedLocalState,
    UnsupportedManifestVersion,
    MalformedManifest,
}

/// Read-only inspection data. The fingerprint is opaque and contains no path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectInspection {
    pub classification: ProjectInspectionClassification,
    pub display_name: String,
    pub warnings: Vec<ProjectInspectionWarning>,
    pub fingerprint: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectWriteErrorCode {
    InvalidProjectName,
    InvalidSceneName,
    SelectionUnavailable,
    InvalidSelection,
    InvalidRootForComposition,
    ProjectAlreadyExists,
    RepositoryUnavailable,
    ManifestUnavailable,
    IncompatibleRepository,
    IgnoreConflict,
    ConcurrentChange,
    Busy,
    RegistrationFailed,
    FilesystemFailure,
}

/// Typed write failures safe to return through the native host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
pub enum ProjectWriteError {
    #[error("unsupported Project write protocol version {actual}; expected {expected}")]
    UnsupportedProtocolVersion { expected: u16, actual: u16 },
    #[error("Project write request is invalid ({code:?})")]
    Invalid { code: ProjectWriteErrorCode },
    #[error("Project write failed ({code:?})")]
    Failed { code: ProjectWriteErrorCode },
    #[error("Project registration failed after valid Project creation")]
    RegistrationFailed { project_created: bool },
    #[error("Project changed after inspection")]
    ConcurrentChange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectCreateRequest {
    pub selection: LocalSelectionToken,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportRequest {
    pub selection: LocalSelectionToken,
    pub inspection: ProjectInspection,
}

/// The stable Project identity selected as the composition target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectWriteTarget {
    Project(ProjectId),
    Scene(SceneId),
    Model(ModelId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectCreateSceneRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneWriteResponse {
    pub project: ProjectSummary,
    pub scene_id: SceneId,
    pub placement_id: Option<SceneMemberId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectAdoptSceneRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub source: LocalSelectionToken,
    pub inspection: CompositionInspection,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneAdoptionResponse {
    pub project: ProjectSummary,
    pub scene_id: SceneId,
    pub placement_id: Option<SceneMemberId>,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportModelRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub source: LocalSelectionToken,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelWriteResponse {
    pub project: ProjectSummary,
    pub model_id: ModelId,
    pub placement_id: Option<SceneMemberId>,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectWriteRequest {
    Inspect { selection: LocalSelectionToken },
    Create(ProjectCreateRequest),
    Import(ProjectImportRequest),
    CreateScene(ProjectCreateSceneRequest),
    AdoptScene(ProjectAdoptSceneRequest),
    ImportModel(ProjectImportModelRequest),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectWriteResponse {
    Inspection(ProjectInspection),
    Project(ProjectSummary),
    Scene(ProjectSceneWriteResponse),
    SceneAdopted(ProjectSceneAdoptionResponse),
    ModelImported(ProjectModelWriteResponse),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectWriteCommand {
    pub protocol_version: u16,
    pub request: ProjectWriteRequest,
}

impl ProjectWriteCommand {
    pub fn new(request: ProjectWriteRequest) -> Self {
        Self {
            protocol_version: PROJECT_WRITE_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectWriteError> {
        if self.protocol_version != PROJECT_WRITE_PROTOCOL_VERSION {
            return Err(ProjectWriteError::UnsupportedProtocolVersion {
                expected: PROJECT_WRITE_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectWriteReply {
    pub protocol_version: u16,
    pub result: Result<ProjectWriteResponse, ProjectWriteError>,
}

impl ProjectWriteReply {
    pub fn success(response: ProjectWriteResponse) -> Self {
        Self {
            protocol_version: PROJECT_WRITE_PROTOCOL_VERSION,
            result: Ok(response),
        }
    }

    pub fn failure(error: ProjectWriteError) -> Self {
        Self {
            protocol_version: PROJECT_WRITE_PROTOCOL_VERSION,
            result: Err(error),
        }
    }
}

impl From<ProjectReadError> for ProjectWriteError {
    fn from(_: ProjectReadError) -> Self {
        Self::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        }
    }
}
