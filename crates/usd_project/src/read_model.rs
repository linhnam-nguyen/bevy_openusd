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
mod tests {
    use super::*;

    fn revision(id: &str) -> RevisionSummary {
        RevisionSummary { id: id.to_owned() }
    }

    #[test]
    fn summary_round_trips_without_adapter_handles() {
        let project = ProjectSummary {
            id: ProjectId::new_v4(),
            name: "Peer view".to_owned(),
            root: ProjectRoot::Scene(SceneId::new_v4()),
            repository: RepositorySummary {
                active_branch: Some("main".to_owned()),
                branches: vec![BranchSummary {
                    name: "main".to_owned(),
                    tip: revision("abc123"),
                    is_current: true,
                }],
                dirty: false,
                head: Some(revision("abc123")),
                latest_commit: Some(CommitSummary {
                    revision: revision("abc123"),
                    subject: "Initial Project".to_owned(),
                    author: "USDHub Test".to_owned(),
                    authored_at_seconds: 0,
                }),
            },
            counts: ProjectContentCounts {
                scenes: 2,
                models: 1,
                scene_placements: 1,
                model_placements: 2,
            },
            issues: ProjectIssueSummary::default(),
            people: ProjectPeopleSummary::default(),
            capabilities: ProjectCapabilities {
                can_create_scene: true,
                can_import_scene: true,
                can_import_model: true,
                can_switch_branch: false,
                can_commit: false,
            },
        };

        let encoded = serde_json::to_string(&project).unwrap();
        let decoded: ProjectSummary = serde_json::from_str(&encoded).unwrap();

        assert_eq!(project, decoded);
    }

    #[test]
    fn repeated_placements_keep_distinct_member_identity() {
        let parent_scene_id = SceneId::new_v4();
        let target_scene_id = SceneId::new_v4();
        let first = SceneMemberId::new_v4();
        let second = SceneMemberId::new_v4();
        let nodes = [
            ProjectContentNode::ScenePlacement {
                member_id: first,
                target: target_scene_id,
                parent_scene_id,
                name: Some("First".to_owned()),
            },
            ProjectContentNode::ScenePlacement {
                member_id: second,
                target: target_scene_id,
                parent_scene_id,
                name: Some("Second".to_owned()),
            },
        ];

        let ids = nodes
            .iter()
            .map(|node| match node {
                ProjectContentNode::ScenePlacement { member_id, .. } => *member_id,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();

        assert_ne!(ids[0], ids[1]);
        assert_ne!(parent_scene_id, target_scene_id);
        assert_eq!(nodes[0], nodes[0]);
    }

    #[test]
    fn optional_issue_provider_does_not_fabricate_zero_counts() {
        let issues = ProjectIssueSummary::default();

        assert_eq!(
            issues.availability,
            ProjectProviderAvailability::NotConfigured
        );
        assert_eq!(issues.issue_count, None);
        assert_eq!(issues.bcf_topic_count, None);
    }

    #[test]
    fn optional_people_provider_does_not_fabricate_zero_count() {
        let people = ProjectPeopleSummary::default();

        assert_eq!(
            people.availability,
            ProjectProviderAvailability::NotConfigured
        );
        assert_eq!(people.participant_count, None);
    }
}
