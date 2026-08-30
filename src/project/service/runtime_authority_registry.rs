use std::{
    env,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use usd_project::ProjectId;

const PROJECT_REGISTRY_PATH_ENV: &str = "USDHUB_PROJECT_WORKSPACE_REGISTRY";

/// Read Project roots from the machine-local workspace registry shared by the
/// Tauri host and render-owner processes. Runtime authority must not depend on
/// a process-local map because those processes have different statics.
pub(crate) fn registered_project_roots(registry_path: Option<&Path>) -> Vec<(ProjectId, PathBuf)> {
    let path = registry_path
        .map(Path::to_path_buf)
        .or_else(|| env::var_os(PROJECT_REGISTRY_PATH_ENV).map(PathBuf::from));
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(registry) = crate::project::catalog::workspace_registry::WorkspaceRegistry::load(path)
    else {
        return Vec::new();
    };
    registry
        .entries()
        .iter()
        .map(|entry| (entry.project_id(), entry.repository_locator().to_owned()))
        .collect()
}

pub(crate) fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis()
}

#[cfg(test)]
mod tests {
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
}
