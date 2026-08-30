use std::{
    collections::HashMap,
    env, fs,
    fs::OpenOptions,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[cfg(test)]
use std::sync::atomic::Ordering;

use super::super::registry;
use super::{
    ProjectRuntimeAuthorityQueue, ProjectRuntimeEnvelope, ProjectRuntimeResponse, runtime_error,
};
use usd_project::ProjectId;
use uuid::Uuid;

pub(super) const RUNTIME_DIRECTORY: &str = ".usdhub/cache/project-runtime-authority";
pub(super) const REQUESTS_DIRECTORY: &str = "requests";
pub(super) const RESPONSES_DIRECTORY: &str = "responses";
pub(super) const CANCELLATIONS_DIRECTORY: &str = "cancellations";
pub(super) const CLAIMS_DIRECTORY: &str = "claims";
const PROJECT_REGISTRY_PATH_ENV: &str = "USDHUB_PROJECT_WORKSPACE_REGISTRY";
const ARTIFACT_RETENTION: Duration = Duration::from_secs(300);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RegistryStamp {
    pub(super) modified: Option<SystemTime>,
    pub(super) length: Option<u64>,
}

#[derive(Default)]
pub(super) struct RegistryCache {
    pub(super) path: Option<PathBuf>,
    pub(super) stamp: Option<RegistryStamp>,
    pub(super) roots: HashMap<ProjectId, PathBuf>,
    #[cfg(test)]
    pub(super) resolution_count: std::sync::atomic::AtomicUsize,
}

pub(super) fn registry_stamp(path: &Path) -> RegistryStamp {
    let metadata = fs::metadata(path).ok();
    RegistryStamp {
        modified: metadata.as_ref().and_then(|value| value.modified().ok()),
        length: metadata.map(|value| value.len()),
    }
}

pub(super) fn workspace_runtime_root(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(RUNTIME_DIRECTORY)
}

pub(super) fn remove_stale_artifacts(root: &Path, retention: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(modified) = entry.metadata().and_then(|value| value.modified()) else {
            continue;
        };
        if now
            .duration_since(modified)
            .map(|age| age > retention)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(path);
        }
    }
}

pub(super) fn cleanup_stale_artifacts(root: &Path, retention: Duration) {
    for directory in [
        REQUESTS_DIRECTORY,
        RESPONSES_DIRECTORY,
        CANCELLATIONS_DIRECTORY,
        CLAIMS_DIRECTORY,
    ] {
        remove_stale_artifacts(&root.join(directory), retention);
    }
}

pub(super) fn runtime_root(queue: &ProjectRuntimeAuthorityQueue, project_root: &Path) -> PathBuf {
    workspace_runtime_root_for_queue(queue).unwrap_or_else(|| project_root.join(RUNTIME_DIRECTORY))
}

pub(super) fn registry_path(queue: &ProjectRuntimeAuthorityQueue) -> Option<PathBuf> {
    queue
        .workspace_registry_path
        .clone()
        .or_else(|| env::var_os(PROJECT_REGISTRY_PATH_ENV).map(PathBuf::from))
}

fn workspace_runtime_root_for_queue(queue: &ProjectRuntimeAuthorityQueue) -> Option<PathBuf> {
    registry_path(queue).map(|path| workspace_runtime_root(&path))
}

pub(super) fn maybe_cleanup(queue: &ProjectRuntimeAuthorityQueue, root: &Path) {
    let now = std::time::Instant::now();
    let mut last_cleanup = queue
        .last_cleanup
        .lock()
        .expect("Project runtime cleanup state is not poisoned");
    if last_cleanup.is_some_and(|last| now.duration_since(last) < CLEANUP_INTERVAL) {
        return;
    }
    cleanup_stale_artifacts(root, ARTIFACT_RETENTION);
    *last_cleanup = Some(now);
}

pub(super) fn consume_pending(
    queue: &ProjectRuntimeAuthorityQueue,
) -> Result<Vec<ProjectRuntimeEnvelope>, project_protocol::ProjectWriteError> {
    let Some(root) = workspace_runtime_root_for_queue(queue) else {
        return Ok(Vec::new());
    };
    maybe_cleanup(queue, &root);
    let _guard = queue
        .file_lock
        .lock()
        .expect("Project runtime authority queue is not poisoned");
    let directory = root.join(REQUESTS_DIRECTORY);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|_| runtime_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| runtime_error())?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut requests = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(|_| runtime_error())?;
        let envelope = serde_json::from_slice(&bytes).map_err(|_| runtime_error())?;
        fs::remove_file(path).map_err(|_| runtime_error())?;
        requests.push(envelope);
    }
    Ok(requests)
}

pub(super) fn registered_project_roots(
    queue: &ProjectRuntimeAuthorityQueue,
) -> HashMap<ProjectId, PathBuf> {
    let Some(path) = registry_path(queue) else {
        return HashMap::new();
    };
    let stamp = registry_stamp(&path);
    let mut cache = queue
        .registry_cache
        .lock()
        .expect("Project runtime registry cache is not poisoned");
    #[cfg(test)]
    cache.resolution_count.fetch_add(1, Ordering::Relaxed);
    if cache.path.as_deref() == Some(path.as_path()) && cache.stamp.as_ref() == Some(&stamp) {
        return cache.roots.clone();
    }
    cache.path = Some(path.clone());
    cache.stamp = Some(stamp);
    cache.roots = registry::load_project_roots(&path).into_iter().collect();
    cache.roots.clone()
}

pub(super) fn is_cancelled(
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    request_id: &str,
) -> bool {
    runtime_root(queue, project_root)
        .join(CANCELLATIONS_DIRECTORY)
        .join(format!("{request_id}.json"))
        .is_file()
}

pub(super) fn create_waiting_claim(
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    request_id: &str,
) -> Result<(), project_protocol::ProjectWriteError> {
    let directory = runtime_root(queue, project_root).join(CLAIMS_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|_| runtime_error())?;
    let path = directory.join(format!("{request_id}.waiting"));
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(runtime_error()),
        Err(_) => Err(runtime_error()),
    }
}

pub(super) fn claim_request(
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    request_id: &str,
) -> Result<bool, project_protocol::ProjectWriteError> {
    let directory = runtime_root(queue, project_root).join(CLAIMS_DIRECTORY);
    match fs::rename(
        directory.join(format!("{request_id}.waiting")),
        directory.join(format!("{request_id}.active")),
    ) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(runtime_error()),
    }
}

pub(super) fn cancel_waiting_request(
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    request_id: &str,
) -> Result<bool, project_protocol::ProjectWriteError> {
    let root = runtime_root(queue, project_root);
    let cancellation_directory = root.join(CANCELLATIONS_DIRECTORY);
    fs::create_dir_all(&cancellation_directory).map_err(|_| runtime_error())?;
    match fs::rename(
        root.join(CLAIMS_DIRECTORY)
            .join(format!("{request_id}.waiting")),
        cancellation_directory.join(format!("{request_id}.json")),
    ) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(runtime_error()),
    }
}

pub(super) fn clear_request_state(
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    request_id: &str,
) {
    let root = runtime_root(queue, project_root);
    for path in [
        root.join(CLAIMS_DIRECTORY)
            .join(format!("{request_id}.waiting")),
        root.join(CLAIMS_DIRECTORY)
            .join(format!("{request_id}.active")),
        root.join(CANCELLATIONS_DIRECTORY)
            .join(format!("{request_id}.json")),
    ] {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn publish_response(
    queue: &ProjectRuntimeAuthorityQueue,
    response: &ProjectRuntimeResponse,
) -> Result<(), project_protocol::ProjectWriteError> {
    let request_id = match response {
        ProjectRuntimeResponse::Ready { request_id, .. }
        | ProjectRuntimeResponse::Finished { request_id }
        | ProjectRuntimeResponse::Validated { request_id }
        | ProjectRuntimeResponse::Renewed { request_id }
        | ProjectRuntimeResponse::Inactive { request_id }
        | ProjectRuntimeResponse::Failed { request_id, .. } => request_id,
    };
    let Some(root) = workspace_runtime_root_for_queue(queue) else {
        return Err(runtime_error());
    };
    maybe_cleanup(queue, &root);
    let directory = root.join(RESPONSES_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|_| runtime_error())?;
    let path = directory.join(format!("{request_id}.json"));
    let temporary = directory.join(format!("{request_id}.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec(response).map_err(|_| runtime_error())?;
    fs::write(&temporary, bytes).map_err(|_| runtime_error())?;
    fs::rename(temporary, path).map_err(|_| runtime_error())
}

#[cfg(test)]
pub(super) fn cleanup_for_test(queue: &ProjectRuntimeAuthorityQueue, project_root: &Path) {
    cleanup_stale_artifacts(&runtime_root(queue, project_root), Duration::ZERO);
}
