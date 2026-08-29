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
    identity: ProjectCacheIdentity,
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
        let identity = match ProjectCacheIdentity::for_project(
            &project_root,
            target.clone(),
            RuntimeProfile::NativeMedium,
            SemanticConfig::default().hash(),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!(
                    "Project cache warm identity could not be established for {}: {error:#}",
                    project_root.display()
                );
                return false;
            }
        };
        let key = (project_root.clone(), identity_key(&identity));
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
            identity,
            key: key.clone(),
        };
        if self.sender.try_send(job).is_err() {
            pending.remove(&key);
            return false;
        }
        true
    }

    /// Enqueue the changed target and every composed ancestor up to the
    /// Project root. This keeps reusable Scene composition cache identities
    /// source-specific without synchronously rebuilding any descriptor.
    pub fn enqueue_affected(&self, project_root: &Path, target: ProjectCacheTarget) -> bool {
        let targets = match affected_targets(project_root, &target) {
            Ok(targets) => targets,
            Err(error) => {
                log::warn!(
                    "Project cache affected-target discovery failed for {}: {error:#}",
                    project_root.display()
                );
                vec![target]
            }
        };
        targets.into_iter().fold(true, |accepted, target| {
            self.enqueue(project_root, target) && accepted
        })
    }

    /// Remove descriptors for a deleted target. Payload objects are immutable
    /// and intentionally remain available for later content-addressed reuse.
    pub fn remove_target_descriptors(
        &self,
        project_root: &Path,
        target: &ProjectCacheTarget,
    ) -> bool {
        match ProjectCacheStore::new(project_root).remove_target(target) {
            Ok(_) => true,
            Err(error) => {
                log::warn!(
                    "Project cache descriptor cleanup failed for {} ({}): {error:#}",
                    project_root.display(),
                    target.key()
                );
                false
            }
        }
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
        let result = warm_job(&job);
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

fn warm_job(job: &WarmJob) -> Result<()> {
    let current_identity = ProjectCacheIdentity::for_project(
        &job.project_root,
        job.target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    if current_identity != job.identity {
        return Ok(());
    }
    let store = ProjectCacheStore::new(&job.project_root);
    if store
        .load(&job.identity)?
        .is_some_and(|descriptor| descriptor.state == ProjectCacheState::Ready)
    {
        return Ok(());
    }
    store.publish(&ProjectCacheDescriptor::new(
        job.identity.clone(),
        ProjectCacheState::Building,
        None,
    )?)?;

    let state = match target_stage_path(&job.project_root, &job.target) {
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
    let latest_identity = ProjectCacheIdentity::for_project(
        &job.project_root,
        job.target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    if latest_identity == job.identity {
        store.publish(&ProjectCacheDescriptor::new(
            job.identity.clone(),
            state,
            None,
        )?)?;
    }
    Ok(())
}

fn identity_key(identity: &ProjectCacheIdentity) -> String {
    let bytes = serde_json::to_vec(identity).expect("Project cache identity is serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

fn affected_targets(
    project_root: &Path,
    target: &ProjectCacheTarget,
) -> Result<Vec<ProjectCacheTarget>> {
    let manifest = ManifestStore::read_validated(project_root)
        .context("read Project manifest for affected cache targets")?;
    let mut scene_ids = HashSet::new();
    let mut model_id = None;
    match target {
        ProjectCacheTarget::ProjectRoot => {}
        ProjectCacheTarget::Scene { id } => {
            scene_ids.insert(id.clone());
        }
        ProjectCacheTarget::Model { id } => {
            model_id = Some(id.as_str());
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for scene in manifest.scenes() {
            let members = crate::project::scene::authoring::read_scene_members(
                &scene_path(project_root, scene.id),
                scene.id,
            )?;
            let contains_changed_model = model_id.is_some_and(|id| {
                members.iter().any(|member| {
                    matches!(member.target, usd_project::SceneMemberTarget::Model(model) if model.to_string() == id)
                })
            });
            let contains_changed_scene = members.iter().any(|member| {
                matches!(member.target, usd_project::SceneMemberTarget::Scene(child) if scene_ids.contains(&child.to_string()))
            });
            if contains_changed_model || contains_changed_scene {
                changed |= scene_ids.insert(scene.id.to_string());
            }
        }
    }

    let mut targets = vec![target.clone()];
    let mut ancestor_ids = scene_ids;
    if let ProjectCacheTarget::Scene { id } = target {
        ancestor_ids.remove(id);
    }
    let mut ancestors = ancestor_ids
        .into_iter()
        .map(|id| ProjectCacheTarget::Scene { id })
        .collect::<Vec<_>>();
    ancestors.sort_by_key(ProjectCacheTarget::key);
    targets.extend(ancestors);
    if !matches!(target, ProjectCacheTarget::ProjectRoot) {
        targets.push(ProjectCacheTarget::ProjectRoot);
    }
    targets.dedup_by_key(|target| target.key());
    Ok(targets)
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

    #[test]
    fn affected_scene_targets_include_composed_ancestors_and_root() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let project_id = usd_project::ProjectId::new_v4();
        let root_scene = usd_project::SceneId::new_v4();
        let child_scene = usd_project::SceneId::new_v4();
        let manifest = usd_project::ProjectManifestV1::new(
            project_id,
            "Warm Project",
            usd_project::ProjectRoot::Scene(root_scene),
            vec![
                usd_project::SceneManifestEntry {
                    id: root_scene,
                    storage_key: usd_project::StorageKey::new("root").unwrap(),
                },
                usd_project::SceneManifestEntry {
                    id: child_scene,
                    storage_key: usd_project::StorageKey::new("child").unwrap(),
                },
            ],
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
        crate::project::scene::authoring::author_scene_atomic_with_members(
            directory.path(),
            root_scene,
            &[usd_project::SceneMember {
                id: usd_project::SceneMemberId::new_v4(),
                target: usd_project::SceneMemberTarget::Scene(child_scene),
                name: None,
                transform: Default::default(),
            }],
        )?;
        crate::project::scene::authoring::author_scene_atomic_with_members(
            directory.path(),
            child_scene,
            &[],
        )?;

        let targets = affected_targets(
            directory.path(),
            &ProjectCacheTarget::Scene {
                id: child_scene.to_string(),
            },
        )?;
        let keys = targets
            .into_iter()
            .map(|target| target.key())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                format!("scene:{child_scene}"),
                format!("scene:{root_scene}"),
                "project".to_owned(),
            ]
        );
        Ok(())
    }

    #[test]
    fn source_stamped_warm_keys_change_with_working_content() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        fs::write(directory.path().join("stage.usda"), b"first")?;
        let first = ProjectCacheIdentity::for_project(
            directory.path(),
            ProjectCacheTarget::ProjectRoot,
            RuntimeProfile::NativeMedium,
            SemanticConfig::default().hash(),
        )?;
        fs::write(directory.path().join("stage.usda"), b"second")?;
        let second = ProjectCacheIdentity::for_project(
            directory.path(),
            ProjectCacheTarget::ProjectRoot,
            RuntimeProfile::NativeMedium,
            SemanticConfig::default().hash(),
        )?;
        assert_ne!(identity_key(&first), identity_key(&second));
        Ok(())
    }
}
