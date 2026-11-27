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

use super::contract::{SceneCacheDescriptorV3, SceneCacheState};
use super::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget, SceneCacheStore,
};
use crate::project::{
    catalog::manifest_store::ManifestStore, models::wrapper::model_wrapper_path,
    scene::authoring::scene_path,
};

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/preparation_tests.rs"]
mod preparation;
mod queue;
mod recovery;
pub(crate) mod runtime;
pub(crate) use queue::ProjectCachePreparation;
pub use queue::ProjectCacheWarmQueue;
pub(crate) use recovery::enqueue_project_targets_fail_closed;

#[derive(Clone)]
struct WarmTarget {
    target: ProjectCacheTarget,
    // Durable desired generation for every managed target (legacy field name).
    scene_generation: Option<u64>,
    build_generation: u64,
    delta_hint: Option<super::changes::SceneDeltaHint>,
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
        // The enqueue owner reads the manifest under the Project transaction
        // and preserves an invalidation barrier on discovery failure.
        self.enqueue_all_targets_for_mutation(project_root)
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
                let _guard = scene_lock
                    .lock()
                    .expect("Scene cache lifecycle lock is not poisoned");
                let had_cache = crate::project::storage::ProjectStorageLayout::new(project_root)
                    .cache_dir()
                    .exists();
                if !had_cache { return Ok(()); }
                super::sync::with_cache_lock(project_root, "retirement", || {
                    SceneCacheStore::new(project_root).retire_scene(scene_id)?;
                    ProjectCacheStore::new(project_root).remove_target(target)?;
                    Ok(())
                })?;
                if had_cache {
                    runtime::publish_current_project_cache_lookup(project_root)?;
                }
            } else {
                ProjectCacheStore::new(project_root).retire_target(target)?;
            }
            Ok(())
        })();
        match cleanup {
            Ok(()) => {
                // The retirement closure (including Scene lifecycle lock) has ended.
                self.schedule_cache_maintenance(project_root);
                true
            }
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

pub(super) fn project_cache_targets(
    manifest: &usd_project::ValidatedProjectManifest,
) -> Vec<ProjectCacheTarget> {
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
    targets
}

fn warm_target(project_root: &Path, target: &WarmTarget) -> Result<()> {
    if let ProjectCacheTarget::Scene { id } = &target.target {
        let generation = target
            .scene_generation
            .context("Scene warm target missing generation")?;
        let scene_id = SceneId::parse(id).context("parse Scene cache target id")?;
        let scene_lock = scene_lifecycle_lock(project_root, scene_id);
        let _guard = scene_lock
            .lock()
            .expect("Scene cache lifecycle lock is not poisoned");
        let scene_store = SceneCacheStore::new(project_root);
        if !scene_store
            .load_descriptor(scene_id)?
            .is_some_and(|descriptor| descriptor.generation == generation)
        {
            return Ok(());
        }
        if super::sync::qualified_cache_root(project_root, false)?.is_none() {
        return Ok(());
    }
    let identity = ProjectCacheIdentity::for_project(
            project_root,
            target.target.clone(),
            RuntimeProfile::NativeMedium,
            super::compatibility::project_runtime_cache_config_hash(
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
        return warm_scene_target(
            project_root,
            id,
            generation,
            &identity,
            target.delta_hint.as_ref(),
        );
    }

    if super::sync::qualified_cache_root(project_root, false)?.is_none() { return Ok(()); }
    let generation = target.scene_generation.context("managed Project target missing desired generation")?;
    let store = ProjectCacheStore::new(project_root);
    let pending = super::sync::PendingIntentStore::new(project_root);
    let Some(desired) = pending.load_record(&target.target)? else { return Ok(()); };
    if desired.generation != generation
        || super::sync::TombstoneStore::new(project_root).is_retired_at(&target.target, generation)? {
        return Ok(());
    }
    let identity = ProjectCacheIdentity::for_project(project_root, target.target.clone(), RuntimeProfile::NativeMedium, desired.config_hash)?;
    let captured = (|| -> Result<_> {
        let path = target_stage_path(project_root, &target.target)?;
        let live = path.as_deref().map(usd_bevy::LiveStage::open_document).transpose()?;
        let revision = identity.revision_digest_from_snapshot(project_root, live.as_ref())?;
        Ok((path, live, revision))
    })();
    let (source_path, live, revision) = match captured {
        Ok(captured) => captured,
        Err(error) => {
            log::warn!("canonical Project target identity could not be qualified: {error:#}");
            store.publish_fallback(
                &ProjectCacheDescriptor::new(identity, ProjectCacheState::FallbackRequired, None)?,
                generation,
            )?;
            return Ok(());
        }
    };
    if desired.desired_revision_identity != Some(revision)
        || desired.desired_source_hash != Some(identity.target_content_hash) {
        // Recovery will establish a newer desired identity. Never acknowledge
        // a different source using this worker's generation.
        return Ok(());
    }
    if store.reuse_generation(&identity, generation, revision)? { return Ok(()); }
    if !store.publish_generation(&ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Building, None)?, generation, revision)? {
        return Ok(());
    }
    // Legacy object pools are deliberately unswept; a build lease also
    // protects shared history and staging from retirement/GC.
    let _pin = super::retention::CachePinGuard::acquire(project_root, Vec::new(), Vec::new())?;
    let (state, runtime) = match live.as_ref() {
        None => (ProjectCacheState::Empty, None),
        Some(live) => match super::preparation::build_runtime_cache_from_snapshot(project_root, live, &identity) {
            Ok(manifest) => (ProjectCacheState::Ready, Some(manifest)),
            Err(error) => {
                log::warn!("canonical Project stage could not be fully warmed: {error:#}");
                (ProjectCacheState::FallbackRequired, None)
            }
        },
    };
    if store.publish_generation(&ProjectCacheDescriptor::new(identity, state, runtime)?, generation, revision)?
        && let Some(path) = source_path.as_deref() {
        super::preparation::prepare_document_assembly(project_root, path);
    }
    Ok(())
}

fn mark_warm_failure_terminal(project_root: &Path, target: &WarmTarget) {
    let ProjectCacheTarget::Scene { id } = &target.target else {
        return;
    };
    let Some(generation) = target.scene_generation else {
        return;
    };
    let Ok(scene_id) = SceneId::parse(id) else {
        return;
    };
    let store = SceneCacheStore::new(project_root);
    let result = (|| -> Result<()> {
        if super::sync::qualified_cache_root(project_root, false)?.is_none() { return Ok(()); }
        store.with_generation_lock(scene_id, || {
            let Some(mut descriptor) = store.load_descriptor(scene_id)? else { return Ok(()); };
            if descriptor.generation == generation && descriptor.state == SceneCacheState::Building {
                descriptor.state = SceneCacheState::FallbackRequired;
                store.publish_descriptor(&descriptor)?;
            }
            Ok(())
        })
    })();
    if let Err(error) = result {
        log::error!("failed to terminalize Scene cache warm failure for {scene_id} generation {generation}: {error:#}");
    }
}

fn warm_scene_target(
    project_root: &Path,
    id: &str,
    generation: u64,
    identity: &ProjectCacheIdentity,
    delta_hint: Option<&super::changes::SceneDeltaHint>,
) -> Result<()> {
    let scene_id = SceneId::parse(id).context("parse Scene cache target id")?;
    let mut descriptor =
        SceneCacheDescriptorV3::invalidated(scene_id, generation, identity.config_hash);
    descriptor.source_content_hash = Some(identity.target_content_hash);
    descriptor.state = SceneCacheState::Partial;
    let _ = runtime::build_and_publish_managed_scene_cache_generation_with_delta(
        project_root,
        &descriptor,
        delta_hint,
    )?;
    Ok(())
}

pub(crate) fn cache_targets_for_changed_paths(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    changed_paths: &[PathBuf],
) -> Vec<ProjectCacheTarget> {
    let layout = crate::project::storage::ProjectStorageLayout::new(project_root);
    let owns =
        |path: &Path, exact: &Path, directory: &Path| path == exact || path.starts_with(directory);
    let relative = |path: PathBuf| path.strip_prefix(project_root).ok().map(Path::to_path_buf);
    let mut targets = Vec::new();
    for scene in manifest.scenes() {
        let scene_path = if manifest.raw().root == usd_project::ProjectRoot::Scene(scene.id) {
            layout.canonical_root_scene_path(&scene.storage_key)
        } else {
            layout.canonical_scene_path(&scene.storage_key)
        };
        let Some(scene_path) = relative(scene_path) else {
            continue;
        };
        let Some(imports) = relative(layout.canonical_scene_import_dir(scene.id)) else {
            continue;
        };
        if changed_paths
            .iter()
            .any(|path| owns(path, &scene_path, &imports))
        {
            targets.push(ProjectCacheTarget::Scene {
                id: scene.id.to_string(),
            });
        }
    }
    for model in manifest.models() {
        let Some(wrapper) = relative(layout.canonical_model_wrapper_path(model)) else {
            continue;
        };
        let Some(imports) = relative(layout.canonical_model_import_dir(model.id)) else {
            continue;
        };
        if changed_paths
            .iter()
            .any(|path| owns(path, &wrapper, &imports))
        {
            targets.push(ProjectCacheTarget::Model {
                id: model.id.to_string(),
            });
        }
    }
    targets.sort_by_key(ProjectCacheTarget::key);
    targets.dedup_by_key(|target| target.key());
    targets
}

/// Expand a set of directly changed targets to include all transitive
/// referencing ancestor Scenes (with cycle protection) plus `ProjectRoot`.
/// If the composition graph cannot be read completely, conservatively falls
/// back to all targets in the manifest so no stale consumer survives.
pub(crate) fn expand_affected_targets(
    project_root: &Path,
    initial: Vec<ProjectCacheTarget>,
) -> Vec<ProjectCacheTarget> {
    let manifest = match ManifestStore::read_validated(project_root) {
        Ok(manifest) => manifest,
        Err(error) => {
            if let Err(barrier_error) = super::sync::set_cache_invalidation_barrier(project_root) {
                log::error!("could not persist cache invalidation barrier: {barrier_error:#}");
            }
            log::warn!("cache dependency discovery unavailable: {error:#}");
            return Vec::new();
        }
    };
    let mut scene_parents: HashMap<SceneId, Vec<SceneId>> = HashMap::new();
    let mut model_parents: HashMap<usd_project::ModelId, Vec<SceneId>> = HashMap::new();
    for scene in manifest.scenes() {
        let path = scene_path(project_root, scene.id);
        let Ok(members) = crate::project::scene::authoring::read_scene_members(&path, scene.id)
        else {
            return project_cache_targets(&manifest);
        };
        for member in members {
            match member.target {
                usd_project::SceneMemberTarget::Scene(child) => {
                    scene_parents.entry(child).or_default().push(scene.id);
                }
                usd_project::SceneMemberTarget::Model(model_id) => {
                    model_parents.entry(model_id).or_default().push(scene.id);
                }
            }
        }
    }

    let mut targets = initial;
    let mut added = Vec::new();
    let mut pending_scenes = Vec::new();
    let mut visited_scenes = std::collections::HashSet::new();
    for target in &targets {
        match target {
            ProjectCacheTarget::Scene { id } => {
                if let Ok(scene_id) = SceneId::parse(id)
                    && visited_scenes.insert(scene_id)
                {
                    pending_scenes.push(scene_id);
                }
            }
            ProjectCacheTarget::Model { id } => {
                if let Ok(model_id) = usd_project::ModelId::parse(id) {
                    for parent in model_parents.get(&model_id).into_iter().flatten().copied() {
                        if visited_scenes.insert(parent) {
                            pending_scenes.push(parent);
                            added.push(ProjectCacheTarget::Scene {
                                id: parent.to_string(),
                            });
                        }
                    }
                }
            }
            ProjectCacheTarget::ProjectRoot => {}
        }
    }
    targets.extend(added);
    while let Some(child_scene) = pending_scenes.pop() {
        for parent in scene_parents.get(&child_scene).into_iter().flatten().copied() {
            if visited_scenes.insert(parent) {
                pending_scenes.push(parent);
                targets.push(ProjectCacheTarget::Scene {
                    id: parent.to_string(),
                });
            }
        }
    }
    if !matches!(manifest.raw().root, usd_project::ProjectRoot::Empty) {
        targets.push(ProjectCacheTarget::ProjectRoot);
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

pub(super) fn target_stage_path(project_root: &Path, target: &ProjectCacheTarget) -> Result<Option<PathBuf>> {
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
#[path = "../../../tests/unit/project/cache/warming/closure_tests.rs"]
mod closure_tests;
#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/legacy_migration_tests.rs"]
mod legacy_migration_tests;
#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/generation_tests.rs"]
mod generation_tests;
#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/recovery_tests.rs"]
mod recovery_tests;
