//! Bounded, backend-owned Project runtime-cache warming.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

use anyhow::{Context, Result, ensure};
use openusd::usd::{InitialLoadSet, Stage};
use usd_semantic::SemanticConfig;
use viewport_protocol::RuntimeProfile;

use super::cache::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget,
};
use crate::project::{
    catalog::manifest_store::ManifestStore, model_wrapper::model_wrapper_path,
    scene::authoring::scene_path,
};
#[cfg(test)]
use std::time::{Duration, Instant};

const WARM_QUEUE_CAPACITY: usize = 2;
struct WarmJob {
    project_root: PathBuf,
    target: ProjectCacheTarget,
    key: (PathBuf, String),
}

/// A bounded queue that coalesces duplicate target warms before they reach the
/// worker. Warm failures are diagnostic and never fail the source mutation.
#[derive(Clone)]
pub struct ProjectCacheWarmQueue {
    sender: mpsc::SyncSender<WarmJob>,
    pending: Arc<Mutex<HashSet<(PathBuf, String)>>>,
}

impl Default for ProjectCacheWarmQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(WARM_QUEUE_CAPACITY);
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let worker_pending = Arc::clone(&pending);
        std::thread::Builder::new()
            .name("usdhub-project-cache-warm".to_owned())
            .spawn(move || worker_loop(receiver, worker_pending))
            .expect("Project cache warm worker must start");
        Self { sender, pending }
    }
}

impl ProjectCacheWarmQueue {
    /// Try to schedule one canonical Project target without blocking the
    /// caller that just published authoritative Project state.
    pub fn enqueue(&self, project_root: &Path, target: ProjectCacheTarget) -> bool {
        let project_root = project_root.to_path_buf();
        let key = (project_root.clone(), target.key());
        let mut pending = self
            .pending
            .lock()
            .expect("Project cache warm state is not poisoned");
        if !pending.insert(key.clone()) {
            return true;
        }
        let job = WarmJob {
            project_root,
            target,
            key: key.clone(),
        };
        if self.sender.try_send(job).is_err() {
            pending.remove(&key);
            return false;
        }
        true
    }

    #[cfg(test)]
    fn wait_for(
        &self,
        project_root: &Path,
        target: &ProjectCacheTarget,
    ) -> Result<Option<ProjectCacheDescriptor>> {
        let identity = ProjectCacheIdentity::for_project(
            project_root,
            target.clone(),
            RuntimeProfile::NativeMedium,
            SemanticConfig::default().hash(),
        )?;
        let store = ProjectCacheStore::new(project_root);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(descriptor) = store.load(&identity)? {
                if descriptor.state != ProjectCacheState::Building {
                    return Ok(Some(descriptor));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn worker_loop(receiver: mpsc::Receiver<WarmJob>, pending: Arc<Mutex<HashSet<(PathBuf, String)>>>) {
    while let Ok(job) = receiver.recv() {
        let result = warm_job(&job.project_root, &job.target);
        if let Err(error) = result {
            log::warn!(
                "Project cache warm failed for {} ({}): {error:#}",
                job.project_root.display(),
                job.target.key()
            );
        }
        pending
            .lock()
            .expect("Project cache warm state is not poisoned")
            .remove(&job.key);
    }
}

fn warm_job(project_root: &Path, target: &ProjectCacheTarget) -> Result<()> {
    let identity = ProjectCacheIdentity::for_project(
        project_root,
        target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let store = ProjectCacheStore::new(project_root);
    store.publish(&ProjectCacheDescriptor::new(
        identity.clone(),
        ProjectCacheState::Building,
        None,
    )?)?;

    let state = match target_stage_path(project_root, target) {
        Ok(None) => ProjectCacheState::Empty,
        Ok(Some(path)) => match open_stage_without_payloads(&path) {
            Ok(()) => ProjectCacheState::Partial,
            Err(error) => {
                log::warn!("canonical Project stage could not be warmed: {error:#}");
                ProjectCacheState::FallbackRequired
            }
        },
        Err(error) => {
            log::warn!("canonical Project target could not be resolved: {error:#}");
            ProjectCacheState::FallbackRequired
        }
    };
    store.publish(&ProjectCacheDescriptor::new(identity, state, None)?)?;
    Ok(())
}

fn target_stage_path(project_root: &Path, target: &ProjectCacheTarget) -> Result<Option<PathBuf>> {
    let manifest = ManifestStore::read_validated(project_root)
        .context("read Project manifest for cache warm")?;
    let path = match target {
        ProjectCacheTarget::ProjectRoot => match &manifest.raw().root {
            usd_project::ProjectRoot::Empty => return Ok(None),
            usd_project::ProjectRoot::Scene(id) => scene_path(project_root, *id),
            usd_project::ProjectRoot::Model(id) => model_wrapper_path(project_root, *id),
        },
        ProjectCacheTarget::Scene { id } => {
            let scene = manifest
                .scenes()
                .iter()
                .find(|scene| scene.id.to_string() == *id)
                .with_context(|| format!("Scene cache target {id} is not in the manifest"))?;
            scene_path(project_root, scene.id)
        }
        ProjectCacheTarget::Model { id } => {
            let model = manifest
                .models()
                .iter()
                .find(|model| model.id.to_string() == *id)
                .with_context(|| format!("Model cache target {id} is not in the manifest"))?;
            model_wrapper_path(project_root, model.id)
        }
    };
    let path = fs::canonicalize(&path)
        .with_context(|| format!("canonicalize Project cache target {}", path.display()))?;
    ensure!(path.is_file(), "Project cache target is not a file");
    Ok(Some(path))
}

fn open_stage_without_payloads(path: &Path) -> Result<()> {
    let path = path
        .to_str()
        .context("canonical Project stage path must be valid UTF-8")?;
    Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path)
        .context("open canonical Project stage for cache warm")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot};

    use super::*;
    use crate::project::catalog::manifest_store::ManifestStore;

    #[test]
    fn empty_project_is_warmed_without_a_stage_open_failure() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Warm Project",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
        let queue = ProjectCacheWarmQueue::default();
        let target = ProjectCacheTarget::ProjectRoot;

        assert!(queue.enqueue(directory.path(), target.clone()));
        let descriptor = queue
            .wait_for(directory.path(), &target)?
            .expect("empty Project warm completes");
        assert_eq!(descriptor.state, ProjectCacheState::Empty);
        Ok(())
    }

    #[test]
    fn duplicate_warm_requests_are_coalesced() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Warm Project",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
        fs::create_dir_all(directory.path().join(".usdhub/cache"))?;
        let queue = ProjectCacheWarmQueue::default();
        let target = ProjectCacheTarget::ProjectRoot;

        assert!(queue.enqueue(directory.path(), target.clone()));
        assert!(queue.enqueue(directory.path(), target));
        Ok(())
    }
}
