//! Pure Project domain contracts shared by Project adapters and read models.

mod capability;
mod composition;
mod composition_inspection;
mod error;
mod id;
mod manifest;
mod read_model;
mod root;
mod scene;

#[cfg(test)]
#[path = "../tests/unit/validation.rs"]
mod validation;

pub use capability::ProjectCapabilities;
pub use composition::{SceneCompositionError, SceneCompositionGraph};
pub use composition_inspection::{
    CompositionClassification, CompositionDiagnostic, CompositionInspection,
    DependencyClassification, DependencyInspection,
};
pub use error::ProjectDomainError;
pub use id::{ModelId, ProjectId, SceneId, SceneMemberId};
pub use manifest::{
    ModelManifestEntry, PROJECT_MANIFEST_SCHEMA_VERSION, ProjectManifestError, ProjectManifestV1,
    SceneManifestEntry, StorageKey, ValidatedProjectManifest,
};
pub use read_model::{
    BranchSummary, CommitSummary, ModelSourceSummary, ProjectContentCounts, ProjectContentNode,
    ProjectIssueSummary, ProjectPeopleSummary, ProjectProviderAvailability, ProjectSceneLinkStatus,
    ProjectSummary, RepositorySummary, RevisionSummary,
};
pub use root::{ModelSourceKind, ProjectRoot};
pub use scene::{
    SceneMember, SceneMemberTarget, ScenePlacementTransform, SourceSpatialConvention, StageUpAxis,
};

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
