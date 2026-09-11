//! Scene-owned prim-count inspection and freshness reconciliation.

use std::path::{Path, PathBuf};

use bevy::prelude::World;

use crate::project::cache::{
    PrimCountPublication, SceneCacheStore, publish_prim_count_if_current,
};
use crate::project::cache_contract::{SceneCacheActivation, SceneCacheDescriptorV3};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::prim_count::{PrimCountFreshness, PrimCountWorker, SubmitStatus};

use super::super::{SceneDerivedMetadata, StageHandle, StageInfo};

const MAX_PRIM_COUNT_ATTEMPTS: u8 = 3;

pub(super) fn initialize_for_uncached(world: &mut World, path: PathBuf) {
    world.insert_resource(SceneDerivedMetadata::uncached(path));
    sync_stage_info(world);
}

pub(super) fn initialize_for_activation(
    world: &mut World,
    path: &Path,
    activation_generation: u64,
    owner: Option<(PathBuf, usd_project::SceneId)>,
    cache: Option<&SceneCacheActivation>,
) {
    let descriptor = owner.as_ref().and_then(|(project_root, scene_id)| {
        match SceneCacheStore::new(project_root).load_descriptor(scene_id.clone()) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                bevy::log::warn!(
                    "[project-cache] Scene descriptor probe failed for {}: {error:#}",
                    scene_id
                );
                None
            }
        }
    });
    world.insert_resource(SceneDerivedMetadata::from_activation(
        path.to_path_buf(),
        activation_generation,
        owner,
        cache,
        descriptor,
    ));
    sync_stage_info(world);
}

pub(super) fn publish_projected_count(world: &mut World, session_id: u64, count: usize) {
    let should_publish = world
        .get_resource::<SceneDerivedMetadata>()
        .is_some_and(|metadata| {
            metadata.session_id == Some(session_id) && !metadata.prim_count_ready
        });
    if !should_publish {
        return;
    }
    let Some(freshness) = world
        .get_resource::<SceneDerivedMetadata>()
        .and_then(SceneDerivedMetadataFreshness::freshness)
    else {
        return;
    };
    match publish_scene_cache_count(world, &freshness, count) {
        CachePublication::Rejected => return,
        CachePublication::Retryable(descriptor) => {
            reconcile_retryable_cache(world, &descriptor);
            return;
        }
        CachePublication::Failed(error) => {
            record_retryable_failure(world, error);
            return;
        }
        CachePublication::NotApplicable | CachePublication::Published(_) => {}
    }
    let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
    if metadata.session_id == Some(freshness.session_id)
        && metadata.scene_id == freshness.scene_id
        && metadata.path == freshness.path
        && metadata.activation_generation == freshness.activation_generation
    {
        metadata.prim_count = count;
        if metadata.cache_project_root.is_some() {
            metadata.prim_count_ready = true;
        }
    }
    drop(metadata);
    sync_stage_info(world);
}

pub(super) fn update(world: &mut World, session_id: u64) {
    let Some(stage_path) = world
        .get_resource::<StageHandle>()
        .map(|handle| handle.path.clone())
    else {
        return;
    };
    let activation_generation = world
        .get_resource::<StageInfo>()
        .map_or(0, |info| info.activation_generation);
    ensure_current_metadata(world, stage_path.clone(), activation_generation);
    world
        .resource_mut::<SceneDerivedMetadata>()
        .bind_session(session_id);
    sync_stage_info(world);

    let results = world
        .get_resource::<PrimCountWorker>()
        .map(PrimCountWorker::drain_results)
        .unwrap_or_default();
    let mut matched_result = false;
    for result in results {
        let matches = world
            .resource::<SceneDerivedMetadata>()
            .freshness_matches(&result.freshness);
        if !matches {
            continue;
        }
        matched_result = true;
        match result.count {
            Ok(count) => {
                let count = count.saturating_sub(1);
                match publish_scene_cache_count(world, &result.freshness, count) {
                    CachePublication::NotApplicable | CachePublication::Published(_) => {
                        let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
                        metadata.prim_count_pending = false;
                        metadata.prim_count = count;
                        metadata.prim_count_ready = true;
                        metadata.prim_count_terminal = false;
                        metadata.prim_count_error = None;
                    }
                    CachePublication::Rejected => {
                        terminalize(
                            world,
                            "Scene cache identity changed before prim-count publication"
                                .to_owned(),
                        );
                    }
                    CachePublication::Retryable(descriptor) => {
                        reconcile_retryable_cache(world, &descriptor);
                    }
                    CachePublication::Failed(error) => {
                        record_retryable_failure(
                            world,
                            format!("Scene cache prim-count publication failed: {error}"),
                        );
                    }
                }
            }
            Err(error) => {
                let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
                metadata.prim_count_pending = false;
                metadata.prim_count_ready = false;
                metadata.prim_count_error = Some(error.clone());
                metadata.prim_count_terminal = !retry_allowed(metadata.prim_count_attempts);
                bevy::log::warn!(
                    "[stage-metadata] asynchronous prim count failed for {} (attempt {} of {}): {error}",
                    metadata.path.display(),
                    metadata.prim_count_attempts,
                    MAX_PRIM_COUNT_ATTEMPTS,
                );
            }
        }
    }
    sync_stage_info(world);
    if matched_result {
        return;
    }

    let should_submit = {
        let metadata = world.resource::<SceneDerivedMetadata>();
        !metadata.prim_count_ready
            && !metadata.prim_count_pending
            && !metadata.prim_count_terminal
            && retry_allowed(metadata.prim_count_attempts)
    };
    if !should_submit {
        return;
    }
    if world.get_resource::<PrimCountWorker>().is_none() {
        match PrimCountWorker::try_new() {
            Ok(worker) => world.insert_resource(worker),
            Err(error) => {
                terminalize(world, format!("prim-count worker unavailable: {error:#}"));
                return;
            }
        }
    }
    let freshness = world.resource::<SceneDerivedMetadata>().freshness();
    let Some(freshness) = freshness else {
        return;
    };
    let status = world.resource::<PrimCountWorker>().submit(freshness);
    match status {
        SubmitStatus::Queued => {
            let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
            metadata.prim_count_pending = true;
            metadata.prim_count_attempts = metadata.prim_count_attempts.saturating_add(1);
        }
        SubmitStatus::Full => {}
        SubmitStatus::Disconnected => {
            terminalize(world, "prim-count worker disconnected".to_owned());
            return;
        }
    }
    sync_stage_info(world);
}

fn ensure_current_metadata(world: &mut World, path: PathBuf, activation_generation: u64) {
    let current = world.get_resource::<SceneDerivedMetadata>().is_some_and(|metadata| {
        metadata.path == path && metadata.activation_generation == activation_generation
    });
    if !current {
        initialize_for_uncached(world, path);
    }
}

fn terminalize(world: &mut World, error: String) {
    let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
    metadata.prim_count_pending = false;
    metadata.prim_count_terminal = true;
    metadata.prim_count_error = Some(error);
    drop(metadata);
    sync_stage_info(world);
}

fn reconcile_retryable_cache(world: &mut World, descriptor: &SceneCacheDescriptorV3) {
    let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
    metadata.cache_descriptor = Some(descriptor.clone());
    metadata.cache_generation = Some(descriptor.generation);
    metadata.cache_state = Some(descriptor.state);
    metadata.prim_count = descriptor.prim_count as usize;
    metadata.prim_count_ready =
        descriptor.prim_count_ready || descriptor.state == crate::project::cache_contract::SceneCacheState::Ready;
    metadata.prim_count_pending = false;
    metadata.prim_count_terminal = false;
    metadata.prim_count_error = Some(
        "Scene cache descriptor appeared during prim-count publication; retrying current session"
            .to_owned(),
    );
    drop(metadata);
    sync_stage_info(world);
}

fn record_retryable_failure(world: &mut World, error: String) {
    let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
    metadata.prim_count_pending = false;
    metadata.prim_count_ready = false;
    metadata.prim_count_terminal = !retry_allowed(metadata.prim_count_attempts);
    metadata.prim_count_error = Some(error);
    drop(metadata);
    sync_stage_info(world);
}

fn sync_stage_info(world: &mut World) {
    let Some(metadata) = world.get_resource::<SceneDerivedMetadata>().cloned() else {
        return;
    };
    let Some(mut info) = world.get_resource_mut::<StageInfo>() else {
        return;
    };
    info.path = metadata.path.to_string_lossy().into_owned();
    info.activation_generation = metadata.activation_generation;
    info.prim_count = metadata.prim_count;
    info.prim_count_ready = metadata.prim_count_ready;
    info.prim_count_pending = metadata.prim_count_pending;
}

fn retry_allowed(attempts: u8) -> bool {
    attempts < MAX_PRIM_COUNT_ATTEMPTS
}

enum CachePublication {
    NotApplicable,
    Published(SceneCacheDescriptorV3),
    Retryable(SceneCacheDescriptorV3),
    Rejected,
    Failed(String),
}

fn publish_scene_cache_count(
    world: &mut World,
    freshness: &PrimCountFreshness,
    count: usize,
) -> CachePublication {
    let (project_root, scene_id, expected, config_hash) = {
        let Some(metadata) = world.get_resource::<SceneDerivedMetadata>() else {
            return CachePublication::Rejected;
        };
        if !metadata.freshness_matches(freshness) {
            return CachePublication::Rejected;
        }
        let (Some(project_root), Some(scene_id)) =
            (metadata.cache_project_root.clone(), metadata.scene_id.clone())
        else {
            return CachePublication::NotApplicable;
        };
        let config_hash = metadata
            .cache_descriptor
            .as_ref()
            .map_or_else(default_project_cache_config_hash, |descriptor| descriptor.config_hash);
        (
            project_root,
            scene_id,
            metadata.cache_descriptor.clone(),
            config_hash,
        )
    };
    let store = SceneCacheStore::new(&project_root);
    match publish_prim_count_if_current(
        &store,
        scene_id,
        expected.as_ref(),
        config_hash,
        count as u64,
    ) {
        Ok(PrimCountPublication::Published(descriptor)) => {
            let mut metadata = world.resource_mut::<SceneDerivedMetadata>();
            if !metadata.freshness_matches(freshness) {
                return CachePublication::Rejected;
            }
            metadata.cache_generation = Some(descriptor.generation);
            metadata.cache_state = Some(descriptor.state);
            metadata.cache_descriptor = Some(descriptor.clone());
            CachePublication::Published(descriptor)
        }
        Ok(PrimCountPublication::RetryableCurrent(descriptor)) => {
            CachePublication::Retryable(descriptor)
        }
        Ok(PrimCountPublication::Stale) => CachePublication::Rejected,
        Err(error) => CachePublication::Failed(format!("{error:#}")),
    }
}

trait SceneDerivedMetadataFreshness {
    fn freshness(&self) -> Option<PrimCountFreshness>;
    fn freshness_matches(&self, freshness: &PrimCountFreshness) -> bool;
}

impl SceneDerivedMetadataFreshness for SceneDerivedMetadata {
    fn freshness(&self) -> Option<PrimCountFreshness> {
        Some(PrimCountFreshness {
            scene_id: self.scene_id.clone(),
            cache_generation: self.cache_generation,
            path: self.path.clone(),
            activation_generation: self.activation_generation,
            session_id: self.session_id?,
        })
    }

    fn freshness_matches(&self, freshness: &PrimCountFreshness) -> bool {
        self.freshness()
            .as_ref()
            .is_some_and(|current| current == freshness)
    }
}

#[cfg(test)]
#[path = "lifecycle_prim_count_tests.rs"]
mod tests;
