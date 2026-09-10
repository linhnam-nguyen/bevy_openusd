//! Managed-mutation cache invalidation and advisory warm admission.

use std::{path::Path, sync::atomic::Ordering};

use anyhow::{Result, anyhow};
use usd_project::SceneId;

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
        self.enqueue_targets_inner(project_root, targets, true)
    }

    fn enqueue_targets_inner(
        &self,
        project_root: &Path,
        mut targets: Vec<ProjectCacheTarget>,
        fail_closed: bool,
    ) -> Result<bool> {
        targets.sort_by_key(ProjectCacheTarget::key);
        targets.dedup_by_key(|target| target.key());
        let config_hash =
            super::super::super::cache_compatibility::project_runtime_cache_config_hash(
                usd_semantic::SemanticConfig::default().hash(),
            );
        let scene_store = SceneCacheStore::new(project_root);
        let sender = self
            .state
            .sender
            .lock()
            .expect("Project cache warm sender is not poisoned")
            .clone();
        let mut accepted = sender.is_some();
        let mut boundary_error = None;
        let mut boundary_failed = false;
        let mut prepared_targets = Vec::with_capacity(targets.len());
        for target in targets {
            let scene_generation = if let ProjectCacheTarget::Scene { id } = &target {
                let scene_id = match SceneId::parse(id) {
                    Ok(scene_id) => scene_id,
                    Err(error) => {
                        log::warn!("Project cache warm received invalid SceneId {id}: {error}");
                        accepted = false;
                        boundary_failed = true;
                        if fail_closed {
                            boundary_error.get_or_insert_with(|| {
                                anyhow!("invalid managed Scene cache target {id}")
                            });
                        }
                        continue;
                    }
                };
                match scene_store.advance_generation(scene_id, config_hash) {
                    Ok(generation) => Some(generation),
                    Err(error) => {
                        log::warn!(
                            "Project cache Scene generation could not advance for {id}: {error:#}"
                        );
                        accepted = false;
                        boundary_failed = true;
                        if fail_closed && !self.remove_target_descriptors(project_root, &target) {
                            boundary_error.get_or_insert_with(|| {
                                anyhow!(
                                    "Scene cache generation boundary for {id} failed ({error:#}) and fail-closed cleanup also failed"
                                )
                            });
                        }
                        continue;
                    }
                }
            } else {
                None
            };
            prepared_targets.push((target, scene_generation));
        }
        if let Some(error) = boundary_error {
            return Err(error);
        }
        if fail_closed && boundary_failed {
            return Ok(false);
        }
        for (target, scene_generation) in prepared_targets {
            let key = (project_root.to_path_buf(), target.key());
            let warm_target = WarmTarget {
                target,
                scene_generation,
                build_generation: self
                    .state
                    .next_build_generation
                    .fetch_add(1, Ordering::Relaxed),
            };
            let Some(sender) = sender.as_ref() else {
                continue;
            };
            accepted &= self.state.latest.record(key, warm_target, sender);
        }
        Ok(accepted)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicU64, mpsc};

    use anyhow::Result;
    use tempfile::tempdir;
    use usd_project::SceneId;

    use super::super::{LatestWarmState, ProjectCacheWarmQueue, WarmQueueState};
    use super::{ProjectCacheTarget, SceneCacheStore};

    #[test]
    fn managed_mutation_establishes_all_scene_boundaries_before_admitting_warm_work() -> Result<()> {
        let directory = tempdir()?;
        let first_scene = SceneId::new_v4();
        let second_scene = SceneId::new_v4();
        let (sender, receiver) = mpsc::sync_channel(4);
        let latest = Arc::new(LatestWarmState::new());
        let queue = ProjectCacheWarmQueue {
            state: Arc::new(WarmQueueState {
                sender: Arc::new(Mutex::new(Some(sender))),
                latest: latest.clone(),
                worker: Mutex::new(None),
                next_build_generation: AtomicU64::new(1),
            }),
        };

        let result = queue.enqueue_targets_for_mutation(
            directory.path(),
            vec![
                ProjectCacheTarget::Scene { id: first_scene.to_string() },
                ProjectCacheTarget::Scene { id: second_scene.to_string() },
                ProjectCacheTarget::Scene { id: "not-a-scene-id".to_owned() },
            ],
        );

        assert!(result.is_err(), "invalid later Scene must remain a fatal managed boundary error");
        assert!(receiver.try_recv().is_err(), "advisory work must wait for every strict boundary");
        assert_eq!(latest.retained_counts(), (0, 0));
        let store = SceneCacheStore::new(directory.path());
        assert_eq!(store.load_descriptor(first_scene)?.expect("first boundary").generation, 1);
        assert_eq!(store.load_descriptor(second_scene)?.expect("second boundary").generation, 1);
        Ok(())
    }
}
