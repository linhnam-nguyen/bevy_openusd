//! Bounded, backend-owned Project runtime-cache warming.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

use anyhow::{Context, Result, ensure};
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

#[path = "cache_preparation.rs"]
mod preparation;
pub(crate) use preparation::ProjectCachePreparation;

const WARM_QUEUE_CAPACITY: usize = 2;
struct WarmTarget {
    target: ProjectCacheTarget,
    identity: ProjectCacheIdentity,
}

struct WarmJob {
    project_root: PathBuf,
    targets: Vec<WarmTarget>,
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
        self.enqueue_targets(project_root, vec![target])
    }

    /// Try to schedule one bounded batch of canonical targets. One import may
    /// therefore warm every Stage-bearing target without filling the queue
    /// with one unbounded request per Scene or Model.
    pub fn enqueue_targets(&self, project_root: &Path, targets: Vec<ProjectCacheTarget>) -> bool {
        let project_root = project_root.to_path_buf();
        let mut targets = targets;
        targets.sort_by_key(ProjectCacheTarget::key);
        targets.dedup_by_key(|target| target.key());
        let config_hash = SemanticConfig::default().hash();
        let mut warm_targets = Vec::with_capacity(targets.len());
        for target in targets {
            let identity = match ProjectCacheIdentity::for_project(
                &project_root,
                target.clone(),
                RuntimeProfile::NativeMedium,
                config_hash,
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
            warm_targets.push(WarmTarget { target, identity });
        }
        if warm_targets.is_empty() {
            return true;
        }
        let batch_key = warm_targets
            .iter()
            .map(|target| identity_key(&target.identity))
            .collect::<Vec<_>>()
            .join(",");
        let key = (project_root.clone(), format!("batch:{batch_key}"));
        let mut pending = self
            .pending
            .lock()
            .expect("Project cache warm state is not poisoned");
        if !pending.insert(key.clone()) {
            return true;
        }
        let job = WarmJob {
            project_root,
            targets: warm_targets,
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
        self.enqueue_targets(project_root, targets)
    }

    /// Enqueue every Stage-bearing target registered by a freshly imported
    /// Project, including its Project root when one is configured.
    pub fn enqueue_project_targets(&self, project_root: &Path) -> bool {
        let manifest = match ManifestStore::read_validated(project_root) {
            Ok(manifest) => manifest,
            Err(error) => {
                log::warn!(
                    "Project cache import target discovery failed for {}: {error:#}",
                    project_root.display()
                );
                return false;
            }
        };
        let mut targets = Vec::with_capacity(manifest.scenes().len() + manifest.models().len() + 1);
        if !matches!(manifest.raw().root, usd_project::ProjectRoot::Empty) {
            targets.push(ProjectCacheTarget::ProjectRoot);
        }
        targets.extend(
            manifest
                .scenes()
                .iter()
                .map(|scene| ProjectCacheTarget::Scene {
                    id: scene.id.to_string(),
                }),
        );
        targets.extend(
            manifest
                .models()
                .iter()
                .map(|model| ProjectCacheTarget::Model {
                    id: model.id.to_string(),
                }),
        );
        self.enqueue_targets(project_root, targets)
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
}

fn worker_loop(receiver: mpsc::Receiver<WarmJob>, pending: Arc<Mutex<HashSet<(PathBuf, String)>>>) {
    while let Ok(job) = receiver.recv() {
        let _ = warm_job(&job);
        pending
            .lock()
            .expect("Project cache warm state is not poisoned")
            .remove(&job.key);
    }
}

fn warm_job(job: &WarmJob) -> Result<()> {
    for target in &job.targets {
        if let Err(error) = warm_target(&job.project_root, target) {
            log::warn!(
                "Project cache warm failed for {} ({}): {error:#}",
                job.project_root.display(),
                target.target.key()
            );
        }
    }
    Ok(())
}

fn warm_target(project_root: &Path, target: &WarmTarget) -> Result<()> {
    let current_identity = ProjectCacheIdentity::for_project(
        project_root,
        target.target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    if current_identity != target.identity {
        return Ok(());
    }
    let store = ProjectCacheStore::new(project_root);
    if store
        .load(&target.identity)?
        .is_some_and(|descriptor| descriptor.state == ProjectCacheState::Ready)
    {
        return Ok(());
    }
    store.publish(&ProjectCacheDescriptor::new(
        target.identity.clone(),
        ProjectCacheState::Building,
        None,
    )?)?;

    let (state, runtime) = match target_stage_path(project_root, &target.target) {
        Ok(None) => (ProjectCacheState::Empty, None),
        Ok(Some(path)) => match super::cache_warm_runtime::build_runtime_cache(
            project_root,
            &path,
            &target.identity,
        ) {
            Ok(manifest) => (ProjectCacheState::Ready, Some(manifest)),
            Err(error) => {
                log::warn!("canonical Project stage could not be fully warmed: {error:#}");
                (ProjectCacheState::FallbackRequired, None)
            }
        },
        Err(error) => {
            log::warn!("canonical Project target could not be resolved: {error:#}");
            (ProjectCacheState::FallbackRequired, None)
        }
    };
    let latest_identity = ProjectCacheIdentity::for_project(
        project_root,
        target.target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    if latest_identity == target.identity {
        store.publish(&ProjectCacheDescriptor::new(
            target.identity.clone(),
            state,
            runtime,
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

#[cfg(test)]
#[path = "cache_warmer_c7_tests.rs"]
mod c7_tests;
#[cfg(test)]
#[path = "cache_warmer_closure_tests.rs"]
mod closure_tests;
#[cfg(test)]
#[path = "cache_warmer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache_warmer_c1_tests.rs"]
mod c1_tests;
