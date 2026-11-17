use tempfile::tempdir;

use super::*;
use crate::project::catalog::workspace_registry::WorkspaceRegistry;

#[test]
fn registered_roots_are_loaded_from_the_shared_workspace_registry() {
    let directory = tempdir().expect("workspace directory");
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let project_root = directory.path().join("project");
    let mut registry = WorkspaceRegistry::load(&registry_path).expect("workspace registry");
    registry
        .register(project_id, &project_root, None)
        .expect("register project root");

    assert_eq!(
        registered_project_roots(Some(&registry_path)),
        vec![(project_id, project_root)]
    );
}
