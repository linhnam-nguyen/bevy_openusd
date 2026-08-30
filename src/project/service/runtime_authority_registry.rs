use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use usd_project::ProjectId;

/// Process-global Project roots let the render owner answer inactive requests.
static REGISTERED_PROJECT_ROOTS: OnceLock<Mutex<HashMap<ProjectId, PathBuf>>> = OnceLock::new();

pub(crate) fn register_project_root(project_id: ProjectId, project_root: &Path) {
    REGISTERED_PROJECT_ROOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Project runtime root registry is not poisoned")
        .insert(project_id, project_root.to_path_buf());
}

pub(crate) fn registered_project_roots() -> Vec<(ProjectId, PathBuf)> {
    REGISTERED_PROJECT_ROOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Project runtime root registry is not poisoned")
        .iter()
        .map(|(project_id, root)| (*project_id, root.clone()))
        .collect()
}

pub(crate) fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis()
}
