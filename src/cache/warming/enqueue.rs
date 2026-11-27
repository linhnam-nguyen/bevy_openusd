//! Managed-mutation cache invalidation and advisory warm admission.

use std::{collections::HashMap, path::Path, sync::atomic::Ordering};

use anyhow::{Result, anyhow};
use usd_project::SceneId;

use super::super::super::changes::SceneDeltaHint;
use super::super::{ProjectCacheTarget, SceneCacheStore, WarmTarget};
use super::ProjectCacheWarmQueue;

impl ProjectCacheWarmQueue {
    pub fn enqueue(&self, project_root: &Path, target: ProjectCacheTarget) -> bool {
        self.enqueue_targets(project_root, vec![target])
    }

    pub fn enqueue_targets(&self, project_root: &Path, targets: Vec<ProjectCacheTarget>) -> bool {
        match self.enqueue_targets_inner(project_root, targets, false) {
            Ok(accepted) => accepted,
            Err(error) => {
                log::warn!(
                    "Project cache warm admission failed for {}: {error:#}",
                    project_root.display()
                );
                false
            }
        }
    }

    /// Establish managed Scene boundaries before admitting advisory warm work.
    /// A boundary failure is returned only when lifecycle-serialized cleanup
    /// also fails; worker absence and queue backpressure remain advisory.
    pub(crate) fn enqueue_targets_for_mutation(
        &self,
        project_root: &Path,
        targets: Vec<ProjectCacheTarget>,
    ) -> Result<bool> {
        self.enqueue_targets_with_scope(project_root, targets, true, false, HashMap::new())
    }

    /// Establish managed Scene boundaries and attach advisory incremental delta
    /// hints keyed by [`ProjectCacheTarget::key`].
    pub(crate) fn enqueue_targets_for_mutation_with_hints(
        &self,
        project_root: &Path,
        targets: Vec<ProjectCacheTarget>,
        delta_hints: HashMap<String, SceneDeltaHint>,
    ) -> Result<bool> {
        self.enqueue_targets_with_scope(project_root, targets, true, false, delta_hints)
    }

    pub(in crate::cache::warming) fn enqueue_all_targets_for_mutation(&self, project_root: &Path) -> Result<bool> {
        self.enqueue_targets_with_scope(project_root, Vec::new(), true, true, HashMap::new())
    }

    fn enqueue_targets_inner(&self, project_root: &Path, targets: Vec<ProjectCacheTarget>, fail_closed: bool) -> Result<bool> {
        self.enqueue_targets_with_scope(project_root, targets, fail_closed, false, HashMap::new())
    }

    fn enqueue_targets_with_scope(
        &self,
        project_root: &Path,
        targets: Vec<ProjectCacheTarget>,
        fail_closed: bool,
        all: bool,
        mut delta_hints: HashMap<String, SceneDeltaHint>,
    ) -> Result<bool> {
        // No queue mutex or Scene lifecycle mutex is taken under this lock.
        // All durable boundaries and the barrier clear are one transaction.
        let Some(prepared) = super::super::super::sync::with_cache_lock(project_root, "invalidate", || {
            Self::prepare_targets_locked(project_root, targets, fail_closed, all)
        })? else { return Ok(false); };
        let sender = self.state.sender.lock().map_err(|_| anyhow!("Project cache warm sender poisoned"))?.clone();
        let Some(sender) = sender.as_ref() else { return Ok(false); };
        let mut accepted = true;
        for (target, generation) in prepared {
            let target_key = target.key();
            let delta_hint = delta_hints.remove(&target_key);
            let key = (project_root.to_path_buf(), target_key);
            let warm_target = WarmTarget {
                target,
                scene_generation: Some(generation),
                build_generation: self.state.next_build_generation.fetch_add(1, Ordering::Relaxed),
                delta_hint,
            };
            accepted &= self.state.latest.record(key, warm_target, sender);
        }
        Ok(accepted)
    }

    fn prepare_targets_locked(project_root: &Path, mut targets: Vec<ProjectCacheTarget>, fail_closed: bool, all: bool) -> Result<Option<Vec<(ProjectCacheTarget, u64)>>> {
        use super::super::super::sync;
        let layout = crate::project::storage::ProjectStorageLayout::new(project_root);
        let manifest = match crate::project::catalog::manifest_store::ManifestStore::read_validated(project_root) {
            Ok(manifest) => Some(manifest),
            Err(error) => {
                // The low-level Scene-only boundary API also serves unregistered
                // standalone fixtures. Registered/all-target recovery cannot
                // treat an absent or malformed manifest as a complete graph.
                let absent = matches!(std::fs::symlink_metadata(layout.readable_manifest_path()),
                    Err(ref error) if error.kind() == std::io::ErrorKind::NotFound);
                if absent && fail_closed && !all && !targets.is_empty() && !sync::cache_invalidation_barrier(project_root)?
                    && targets.iter().all(|target| matches!(target, ProjectCacheTarget::Scene { .. })) {
                    None
                } else {
                    sync::set_cache_invalidation_barrier(project_root)?;
                    log::warn!("cache graph unavailable; Project cache invalidated: {error:#}");
                    return Ok(None);
                }
            }
        };
        let reconcile = sync::cache_invalidation_barrier(project_root)?;
        if let Some(manifest) = manifest.as_ref() {
            let registered = super::super::project_cache_targets(manifest);
            if reconcile || all { targets = registered.clone(); }
            targets.retain(|target| registered.contains(target)
                || matches!(target, ProjectCacheTarget::ProjectRoot)
                || (fail_closed && matches!(target, ProjectCacheTarget::Scene { id } if SceneId::parse(id).is_err())));
        }
        targets.sort_by_key(ProjectCacheTarget::key);
        targets.dedup_by_key(|target| target.key());
        let config_hash = super::super::super::compatibility::project_runtime_cache_config_hash(usd_semantic::SemanticConfig::default().hash());
        let scene_store = SceneCacheStore::new(project_root);
        let project_store = super::super::ProjectCacheStore::new(project_root);
        let mut prepared = Vec::with_capacity(targets.len());
        let mut boundary_error = None;
        let mut invalid_target = false;
        for target in targets {
            invalid_target |= matches!(&target, ProjectCacheTarget::Scene { id } if SceneId::parse(id).is_err());
            let result = match &target {
                ProjectCacheTarget::Scene { id } => SceneId::parse(id)
                    .map_err(|error| anyhow!("invalid managed Scene cache target {id}: {error}"))
                    .and_then(|scene_id| scene_store.advance_generation(scene_id, config_hash)),
                _ => project_store.advance_generation(&target, config_hash),
            };
            match result {
                Ok(generation) => prepared.push((target, generation)),
                Err(error) => {
                    log::warn!("cache generation boundary failed for {}: {error:#}", target.key());
                    boundary_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = boundary_error {
            // Never retire a still-registered target merely because its
            // invalidation failed. Keep the Project ineligible until replay.
            sync::set_cache_invalidation_barrier(project_root)?;
            if fail_closed && invalid_target { return Err(error); }
            return Ok(None);
        }
        if reconcile && manifest.is_some() {
            sync::clear_cache_invalidation_barrier(project_root)?;
        }
        Ok(Some(prepared))
    }

}

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/enqueue_tests.rs"]
mod tests;
