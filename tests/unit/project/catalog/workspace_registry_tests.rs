use std::fs;

use tempfile::tempdir;
use usd_project::{
    ProjectCapabilities, ProjectContentCounts, ProjectRoot, ProjectSummary, RepositorySummary,
};

use super::*;

#[test]
fn register_update_remove_and_restart_preserve_one_entry_per_project() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let first = ProjectId::new_v4();
    let second = ProjectId::new_v4();
    let first_locator = directory.path().join("first");
    let updated_locator = directory.path().join("first-updated");

    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(first, &first_locator, Some(10)).unwrap();
    registry
        .register(second, directory.path().join("second"), None)
        .unwrap();
    registry
        .register(first, &updated_locator, Some(20))
        .unwrap();

    assert_eq!(registry.entries().len(), 2);
    assert_eq!(
        registry.get(first).unwrap().repository_locator(),
        updated_locator
    );
    assert_eq!(registry.get(first).unwrap().last_opened_ms(), Some(20));

    assert!(registry.remove(second).unwrap());
    assert!(!registry.remove(second).unwrap());

    let restarted = WorkspaceRegistry::load(&registry_path).unwrap();
    assert_eq!(restarted.entries().len(), 1);
    assert_eq!(
        restarted.get(first).unwrap().repository_locator(),
        updated_locator
    );
    assert!(
        fs::read_to_string(registry_path)
            .unwrap()
            .contains("schema_version")
    );
}

#[test]
fn duplicate_project_ids_are_rejected_on_load() {
    let directory = tempdir().unwrap();
    let project_id = ProjectId::new_v4();
    let stored = WorkspaceRegistryFile {
        schema_version: WORKSPACE_REGISTRY_SCHEMA_VERSION,
        entries: vec![
            WorkspaceProjectEntry {
                project_id,
                content_project_id: None,
                repository_locator: PathBuf::from("one"),
                last_opened_ms: None,
            },
            WorkspaceProjectEntry {
                project_id,
                content_project_id: None,
                repository_locator: PathBuf::from("two"),
                last_opened_ms: None,
            },
        ],
    };
    fs::write(
        directory.path().join("workspace.json"),
        serde_json::to_vec(&stored).unwrap(),
    )
    .unwrap();

    assert!(WorkspaceRegistry::load(directory.path().join("workspace.json")).is_err());
}

#[test]
fn project_summary_serialization_does_not_include_machine_local_locator() {
    let local_locator = "/machine-local/private/project";
    let summary = ProjectSummary {
        id: ProjectId::new_v4(),
        name: "Project".to_owned(),
        root: ProjectRoot::Empty,
        repository: RepositorySummary {
            active_branch: None,
            branches: Vec::new(),
            dirty: false,
            head: None,
            latest_commit: None,
        },
        counts: ProjectContentCounts::default(),
        issues: usd_project::ProjectIssueSummary::default(),
        people: usd_project::ProjectPeopleSummary::default(),
        capabilities: ProjectCapabilities::default(),
    };

    let encoded = serde_json::to_string(&summary).unwrap();
    assert!(!encoded.contains(local_locator));
}
