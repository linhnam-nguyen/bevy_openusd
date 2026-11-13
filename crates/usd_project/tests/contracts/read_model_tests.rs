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
