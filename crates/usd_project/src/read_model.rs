use serde::{Deserialize, Serialize};

use crate::{
    ModelId, ModelSourceKind, ProjectCapabilities, ProjectId, ProjectRoot, SceneId, SceneMemberId,
};

/// Safe summary state exposed by the Project application boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub name: String,
    pub root: ProjectRoot,
    pub repository: RepositorySummary,
    pub counts: ProjectContentCounts,
    pub issues: ProjectIssueSummary,
    pub people: ProjectPeopleSummary,
    pub capabilities: ProjectCapabilities,
}

/// Availability of an optional Project summary provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectProviderAvailability {
    Available,
    #[default]
    NotConfigured,
    Unavailable,
}

/// Read-only Issue and BCF counts. `None` is intentional when no provider
/// exists; it must not be presented as an authoritative zero.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectIssueSummary {
    pub availability: ProjectProviderAvailability,
    pub issue_count: Option<u64>,
    pub bcf_topic_count: Option<u64>,
}

/// Read-only people summary. Identity and membership management remain
/// outside this milestone; `None` preserves an absent provider explicitly.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectPeopleSummary {
    pub availability: ProjectProviderAvailability,
    pub participant_count: Option<u64>,
}

/// Git-neutral repository state for a Project summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositorySummary {
    pub active_branch: Option<String>,
    pub branches: Vec<BranchSummary>,
    pub dirty: bool,
    pub head: Option<RevisionSummary>,
    pub latest_commit: Option<CommitSummary>,
}

/// One local branch represented without a Git implementation type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BranchSummary {
    pub name: String,
    pub tip: RevisionSummary,
    pub is_current: bool,
}

/// Opaque revision metadata safe to send across the Project boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevisionSummary {
    pub id: String,
}

/// Git-neutral metadata for the current HEAD commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitSummary {
    pub revision: RevisionSummary,
    pub subject: String,
    pub author: String,
    pub authored_at_seconds: i64,
}

/// Bounded counts for Project content without copying the content catalogue.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectContentCounts {
    pub scenes: u64,
    pub models: u64,
    pub scene_placements: u64,
    pub model_placements: u64,
}

/// Derived status of a linked Scene source. This is never used as Project
/// authority; it is a read-only indication for the Projects view.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSceneLinkStatus {
    InSync,
    SourceUnavailable,
    OutOfSync,
}

/// A safe, flat Project content node suitable for paging or tree projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectContentNode {
    Scene {
        scene_id: SceneId,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link_status: Option<ProjectSceneLinkStatus>,
    },
    Model {
        model_id: ModelId,
        name: String,
        source: ModelSourceSummary,
    },
    ScenePlacement {
        member_id: SceneMemberId,
        target: SceneId,
        parent_scene_id: SceneId,
        name: Option<String>,
    },
    ModelPlacement {
        member_id: SceneMemberId,
        target: ModelId,
        parent_scene_id: SceneId,
        name: Option<String>,
    },
}

/// Public model-source metadata derived from the pure source-kind contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelSourceSummary {
    pub kind: ModelSourceKind,
}

#[cfg(test)]
#[path = "../tests/contracts/read_model_tests.rs"]
mod tests;
