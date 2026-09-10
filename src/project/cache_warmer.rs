//! Bounded, backend-owned Project runtime-cache warming.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};

use anyhow::{Context, Result, ensure};
use usd_project::SceneId;
use viewport_protocol::RuntimeProfile;

use super::cache::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget, SceneCacheStore,
};
use super::cache_contract::{SceneCacheDescriptorV3, SceneCacheState};
use crate::project::{
    catalog::manifest_store::ManifestStore, model_wrapper::model_wrapper_path,
    scene::authoring::scene_path,
};

#[path = "cache_warmer_queue.rs"]
mod queue;
#[path = "cache_warmer_recovery.rs"]
mod recovery;
#[cfg(test)]
mod preparation {
    pub(super) use super::queue::wait_for;
}
pub(crate) use queue::ProjectCachePreparation;
pub use queue::ProjectCacheWarmQueue;
pub(crate) use recovery::enqueue_project_targets_fail_closed;

#[derive(Clone)]
struct WarmTarget {
    target: ProjectCacheTarget,
    scene_generation: Option<u64>,
    build_generation: u64,
}

struct WarmJob {
    key: (PathBuf, String),
}

static SCENE_LIFECYCLE_LOCKS: LazyLock<Mutex<HashMap<(PathBuf, SceneId), Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn scene_lifecycle_lock(project_root: &Path, scene_id: SceneId) -> Arc<Mutex<()>> {
    let key = (project_root.to_path_buf(), scene_id);
    let mut locks = SCENE_LIFECYCLE_LOCKS
        .lock()
        .expect("Scene cache lifecycle lock registry is not poisoned");
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

impl ProjectCacheWarmQueue {
    /// Enqueue the changed owned target plus the legacy Project-root target.
    /// Direct-parent composition invalidation is supplied by mutation services
    /// that own the corresponding placement/reparent semantics.
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
        self.enqueue_targets(project_root, project_cache_targets(&manifest))
    }

    pub(crate) fn enqueue_project_targets_for_mutation(&self, project_root: &Path) -> Result<bool> {
        let manifest = ManifestStore::read_validated(project_root)
            .context("read Project manifest for managed cache invalidation")?;
        self.enqueue_targets_for_mutation(project_root, project_cache_targets(&manifest))
    }

    /// Remove all derived state for a deleted target. Scene-owned V3 payloads
    /// are deleted with their SceneId while unrelated Scene caches remain intact.
    pub fn remove_target_descriptors(
        &self,
        project_root: &Path,
        target: &ProjectCacheTarget,
    ) -> bool {
        self.cancel_target(project_root, target);
        let cleanup = (|| -> Result<()> {
            if let ProjectCacheTarget::Scene { id } = target {
                let scene_id = SceneId::parse(id).context("parse deleted Scene cache target id")?;
                let scene_lock = scene_lifecycle_lock(project_root, scene_id);
                let _guard = scene_lock.lock().expect("Scene cache lifecycle lock is not poisoned");
                ProjectCacheStore::new(project_root).remove_target(target)?;
                SceneCacheStore::new(project_root).remove_scene(scene_id)?;
                super::cache_warm_runtime::publish_current_project_cache_lookup(project_root)?;
            } else {
                ProjectCacheStore::new(project_root).remove_target(target)?;
            }
            Ok(())
        })();
        match cleanup {
            Ok(()) => true,
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

fn project_cache_targets(
    manifest: &usd_project::ValidatedProjectManifest,
) -> Vec<ProjectCacheTarget> {
    let mut targets = Vec::with_capacity(manifest.scenes().len() + manifest.models().len() + 1);
    if !matches!(manifest.raw().root, usd_project::ProjectRoot::Empty) {
        targets.push(ProjectCacheTarget::ProjectRoot);
    }
    targets.extend(manifest.scenes().iter().map(|scene| ProjectCacheTarget::Scene {
        id: scene.id.to_string(),
    }));
    targets.extend(manifest.models().iter().map(|model| ProjectCacheTarget::Model {
        id: model.id.to_string(),
    }));
    targets
}

fn warm_target(project_root: &Path, target: &WarmTarget) -> Result<()> {
    if let ProjectCacheTarget::Scene { id } = &target.target {
        let generation = target.scene_generation.context("Scene warm target missing generation")?;
        let scene_id = SceneId::parse(id).context("parse Scene cache target id")?;
        let scene_lock = scene_lifecycle_lock(project_root, scene_id);
        let _guard = scene_lock.lock().expect("Scene cache lifecycle lock is not poisoned");
        let scene_store = SceneCacheStore::new(project_root);
        if !scene_store
            .load_descriptor(scene_id)?
            .is_some_and(|descriptor| descriptor.generation == generation)
        {
            return Ok(());
        }
        let identity = ProjectCacheIdentity::for_project(
            project_root,
            target.target.clone(),
            RuntimeProfile::NativeMedium,
            super::cache_compatibility::project_runtime_cache_config_hash(
                usd_semantic::SemanticConfig::default().hash(),
            ),
        )?;
        if !scene_store
            .load_descriptor(scene_id)?
            .is_some_and(|descriptor| descriptor.generation == generation)
        {
            return Ok(());
        }
        log::debug!(
            "[project-loading] scene_cache_build scene={id} generation={generation} build_generation={}",
            target.build_generation
        );
        return warm_scene_target(project_root, id, generation, &identity);
    }

    let identity = ProjectCacheIdentity::for_project(
        project_root,
        target.target.clone(),
        RuntimeProfile::NativeMedium,
        super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    let store = ProjectCacheStore::new(project_root);
    if store
        .load(&identity)?
        .is_some_and(|descriptor| descriptor.state == ProjectCacheState::Ready)
    {
        return Ok(());
    }
    store.publish(&ProjectCacheDescriptor::new(
        identity.clone(),
        ProjectCacheState::Building,
        None,
    )?)?;

    let (state, runtime) = match target_stage_path(project_root, &target.target) {
        Ok(None) => (ProjectCacheState::Empty, None),
        Ok(Some(path)) => match super::cache_preparation::build_runtime_cache(project_root, &path, &identity) {
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
        super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    if latest_identity == identity {
        store.publish(&ProjectCacheDescriptor::new(identity, state, runtime)?)?;
    }
    Ok(())
}

fn warm_scene_target(
    project_root: &Path,
    id: &str,
    generation: u64,
    identity: &ProjectCacheIdentity,
) -> Result<()> {
    let scene_id = SceneId::parse(id).context("parse Scene cache target id")?;
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, identity.config_hash);
    descriptor.source_content_hash = Some(identity.target_content_hash);
    descriptor.state = SceneCacheState::Partial;
    let _ = super::cache_warm_runtime::build_and_publish_managed_scene_cache_generation(
        project_root,
        &descriptor,
    )?;
    Ok(())
}

pub(crate) fn cache_targets_for_changed_paths(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    changed_paths: &[PathBuf],
) -> Vec<ProjectCacheTarget> {
    let layout = crate::project::storage::ProjectStorageLayout::new(project_root);
    let owns = |path: &Path, exact: &Path, directory: &Path| {
        path == exact || path.starts_with(directory)
    };
    let relative = |path: PathBuf| path.strip_prefix(project_root).ok().map(Path::to_path_buf);
    let mut targets = Vec::new();
    for scene in manifest.scenes() {
        let scene_path = if manifest.raw().root == usd_project::ProjectRoot::Scene(scene.id) {
            layout.canonical_root_scene_path(&scene.storage_key)
        } else {
            layout.canonical_scene_path(&scene.storage_key)
        };
        let Some(scene_path) = relative(scene_path) else { continue; };
        let Some(imports) = relative(layout.canonical_scene_import_dir(scene.id)) else { continue; };
        if changed_paths.iter().any(|path| owns(path, &scene_path, &imports)) {
            targets.push(ProjectCacheTarget::Scene { id: scene.id.to_string() });
        }
    }
    for model in manifest.models() {
        let Some(wrapper) = relative(layout.canonical_model_wrapper_path(model)) else { continue; };
        let Some(imports) = relative(layout.canonical_model_import_dir(model.id)) else { continue; };
        if changed_paths.iter().any(|path| owns(path, &wrapper, &imports)) {
            targets.push(ProjectCacheTarget::Model { id: model.id.to_string() });
        }
    }
    targets.sort_by_key(ProjectCacheTarget::key);
    targets.dedup_by_key(|target| target.key());
    targets
}

fn affected_targets(
    _project_root: &Path,
    target: &ProjectCacheTarget,
) -> Result<Vec<ProjectCacheTarget>> {
    let mut targets = vec![target.clone()];
    if !matches!(target, ProjectCacheTarget::ProjectRoot) {
        targets.push(ProjectCacheTarget::ProjectRoot);
    }
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
#[cfg(test)]
#[path = "cache_warmer_recovery_tests.rs"]
mod recovery_tests;
#[cfg(test)]
#[path = "cache_warmer_lifecycle_tests.rs"]
mod lifecycle_tests;
