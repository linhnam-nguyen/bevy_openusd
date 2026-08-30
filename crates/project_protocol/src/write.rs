use serde::{Deserialize, Serialize};
use usd_project::{
    CompositionInspection, ModelId, ProjectContentCounts, ProjectContentNode, ProjectId,
    ProjectSummary, RepositorySummary, RevisionSummary, SceneId, SceneMemberId,
};

use crate::{LocalSelectionToken, PlacementSpec, ProjectImportProgress, ProjectReadError};

/// Version of the shared Project write command boundary.
pub const PROJECT_WRITE_PROTOCOL_VERSION: u16 = 7;

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
    InvalidModelName,
    InvalidPlacement,
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
    InvalidBranchName,
    BranchNotFound,
    DirtyWorkingTree,
    BranchProjectInvalid,
    BranchSwitchFailed,
    ProjectNotFound,
    ProtectedProjectPath,
    ProjectDeleteFailed,
    ProjectDeleteCleanupFailed,
    ProjectRemoveFailed,
    SceneNotFound,
    ProtectedRootScene,
    SceneInUse,
    SceneDeleteFailed,
    SceneDeleteCleanupFailed,
    ScenePlacementNotFound,
    ScenePlacementRemoveFailed,
    CommitMessageInvalid,
    NothingToCommit,
    CommitFailed,
    ExportDestinationInvalid,
    ExportFailed,
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
    #[error("target branch contains invalid Project metadata")]
    BranchProjectInvalid { repository: Box<RepositorySummary> },
    #[error("repository truth unavailable after branch checkout")]
    BranchProjectTruthUnavailable,
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
pub struct ProjectRenameRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectRenameResponse {
    pub project: ProjectSummary,
    pub target: ProjectWriteTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneWriteResponse {
    pub project: ProjectSummary,
    pub scene_id: SceneId,
    pub placement_id: Option<SceneMemberId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectAdoptSceneRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub source: LocalSelectionToken,
    pub inspection: CompositionInspection,
    pub name: String,
    pub operation_id: String,
    pub generation: u64,
    pub placement: PlacementSpec,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectLinkSceneRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub source: LocalSelectionToken,
    pub inspection: CompositionInspection,
    pub name: String,
    pub operation_id: String,
    pub generation: u64,
    pub placement: PlacementSpec,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectSyncLinkedSceneRequest {
    pub project_id: ProjectId,
    pub scene_id: SceneId,
    pub source: LocalSelectionToken,
    pub inspection: CompositionInspection,
    pub name: String,
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
    pub progress: ProjectImportProgress,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectImportModelRequest {
    pub project_id: ProjectId,
    pub target: ProjectWriteTarget,
    pub source: LocalSelectionToken,
    pub operation_id: String,
    pub generation: u64,
    pub placement: PlacementSpec,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectBranchSwitchRequest {
    pub project_id: ProjectId,
    pub branch_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectBranchSwitchResponse {
    pub project: ProjectSummary,
    pub repository: RepositorySummary,
    pub nodes: Vec<ProjectContentNode>,
    pub counts: ProjectContentCounts,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectLifecycleRequest {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectLifecycleResponse {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectCommitTarget {
    Project,
    Scene(SceneId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectCommitRequest {
    pub project_id: ProjectId,
    pub target: ProjectCommitTarget,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectCommitResponse {
    pub project: ProjectSummary,
    pub repository: RepositorySummary,
    pub revision: RevisionSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectExportSceneRequest {
    pub project_id: ProjectId,
    pub scene_id: SceneId,
    pub destination: LocalSelectionToken,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneExportResponse {
    pub project_id: ProjectId,
    pub scene_id: SceneId,
    pub file_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectRemoveScenePlacementRequest {
    pub project_id: ProjectId,
    pub parent_scene_id: SceneId,
    pub placement_id: SceneMemberId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectDeleteSceneRequest {
    pub project_id: ProjectId,
    pub scene_id: SceneId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectDeleteModelRequest {
    pub project_id: ProjectId,
    pub model_id: ModelId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSceneLifecycleResponse {
    pub project_id: ProjectId,
    pub scene_id: SceneId,
    pub placement_id: Option<SceneMemberId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelLifecycleResponse {
    pub project_id: ProjectId,
    pub model_id: ModelId,
    pub placement_ids: Vec<SceneMemberId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectModelWriteResponse {
    pub project: ProjectSummary,
    pub model_id: ModelId,
    pub placement_id: Option<SceneMemberId>,
    pub operation_id: String,
    pub generation: u64,
    pub progress: ProjectImportProgress,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ProjectWriteRequest {
    Inspect { selection: LocalSelectionToken },
    Create(ProjectCreateRequest),
    Import(ProjectImportRequest),
    CreateScene(ProjectCreateSceneRequest),
    Rename(ProjectRenameRequest),
    AdoptScene(ProjectAdoptSceneRequest),
    LinkScene(ProjectLinkSceneRequest),
    SyncLinkedScene(ProjectSyncLinkedSceneRequest),
    ImportModel(ProjectImportModelRequest),
    SwitchBranch(ProjectBranchSwitchRequest),
    RemoveProject(ProjectLifecycleRequest),
    DeleteProject(ProjectLifecycleRequest),
    RemoveScenePlacement(ProjectRemoveScenePlacementRequest),
    DeleteScene(ProjectDeleteSceneRequest),
    DeleteModel(ProjectDeleteModelRequest),
    CommitProject(ProjectCommitRequest),
    CommitScene(ProjectCommitRequest),
    ExportScene(ProjectExportSceneRequest),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectWriteResponse {
    Inspection(ProjectInspection),
    Project(ProjectSummary),
    Scene(ProjectSceneWriteResponse),
    Renamed(ProjectRenameResponse),
    SceneAdopted(ProjectSceneAdoptionResponse),
    SceneLinked(ProjectSceneAdoptionResponse),
    SceneLinkSynced(ProjectSceneAdoptionResponse),
    ModelImported(ProjectModelWriteResponse),
    BranchSwitched(ProjectBranchSwitchResponse),
    ProjectRemoved(ProjectLifecycleResponse),
    ProjectDeleted(ProjectLifecycleResponse),
    ScenePlacementRemoved(ProjectSceneLifecycleResponse),
    SceneDeleted(ProjectSceneLifecycleResponse),
    ModelDeleted(ProjectModelLifecycleResponse),
    Committed(ProjectCommitResponse),
    SceneExported(ProjectSceneExportResponse),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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
