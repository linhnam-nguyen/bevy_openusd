//! Shared, adapter-neutral Project read protocol.
//!
//! This crate is intentionally limited to serde data transfer objects. It
//! does not know about Tauri, Git implementations, filesystems, OpenUSD, or
//! renderer state.

mod activation;
mod command;
mod error;
mod location;
mod model_preparation;
mod placement;
mod progress;
mod read;
mod scene_inspection;
mod write;

pub use activation::{
    PROJECT_ACTIVATION_PROTOCOL_VERSION, ProjectActivationCommand, ProjectActivationError,
    ProjectActivationReply, ProjectActivationResult, ProjectStageTarget,
};
pub use command::{PROJECT_READ_PROTOCOL_VERSION, ProjectReadCommand, ProjectReadReply};
pub use error::{ProjectReadError, ProjectReadErrorCode};
pub use location::{
    LocalSelectionToken, LocalSelectionView, ProjectLocationKind, ProjectLocationResult,
};
pub use model_preparation::{
    PROJECT_MODEL_PREPARATION_PROTOCOL_VERSION, ProjectModelPreparationCommand,
    ProjectModelPreparationReply, ProjectModelPreparationRequest, ProjectModelPreparationResult,
};
pub use placement::{PlacementSpec, PlacementValidationError};
pub use progress::{
    PROJECT_IMPORT_PROGRESS_PROTOCOL_VERSION, ProjectImportPhase, ProjectImportProgress,
    ProjectImportProgressCommand, ProjectImportProgressReply, ProjectImportProgressRequest,
};
pub use read::{ProjectListItem, ProjectReadRequest, ProjectReadResponse};
pub use scene_inspection::{
    PROJECT_SCENE_INSPECTION_PROTOCOL_VERSION, ProjectSceneInspectionCommand,
    ProjectSceneInspectionReply, ProjectSceneInspectionRequest, ProjectSceneInspectionResult,
};
pub use write::{
    PROJECT_WRITE_PROTOCOL_VERSION, ProjectAdoptSceneRequest, ProjectBranchSwitchRequest,
    ProjectBranchSwitchResponse, ProjectCommitRequest, ProjectCommitResponse, ProjectCommitTarget,
    ProjectCreateRequest, ProjectCreateSceneRequest, ProjectDeleteModelRequest,
    ProjectDeleteSceneRequest, ProjectExportSceneRequest, ProjectImportModelRequest,
    ProjectImportRequest, ProjectInspection, ProjectInspectionClassification,
    ProjectInspectionWarning, ProjectLifecycleRequest, ProjectLifecycleResponse,
    ProjectLinkSceneRequest, ProjectModelLifecycleResponse, ProjectModelWriteResponse,
    ProjectRenameRequest, ProjectRenameResponse, ProjectSceneAdoptionResponse,
    ProjectSceneExportResponse, ProjectSceneLifecycleResponse, ProjectSceneWriteResponse,
    ProjectSyncLinkedSceneRequest, ProjectWriteCommand, ProjectWriteError, ProjectWriteErrorCode,
    ProjectWriteReply, ProjectWriteRequest, ProjectWriteResponse, ProjectWriteTarget,
};
