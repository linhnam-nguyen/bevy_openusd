//! Headless Scene-owned cache building plus legacy non-Scene runtime warming.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_project::SceneId;
use uuid::Uuid;
use viewport_protocol::RuntimeManifest;

use super::cache::{ProjectCacheIdentity, SceneCacheDescriptorV3, SceneCacheStore};
use super::cache_contract::{
    CacheObjectRef, PROJECT_CACHE_INDEX_SCHEMA_VERSION, ProjectCacheLookup, SceneCacheAddress,
    SceneCacheIndex, SceneCacheState,
};
pub(crate) use super::cache_scene_build::{
    assign_parent_indexes, build_scene_cache_index, build_spatial_index,
};

#[derive(Deserialize, Serialize)]
struct ProjectCacheIndexV2 {
    schema_version: u16,
    generations: Vec<(SceneId, u64)>,
    rows: Vec<(SceneCacheAddress, CacheObjectRef)>,
}

pub(super) fn build_runtime_cache(
    project_root: &Path,
    path: &Path,
    identity: &ProjectCacheIdentity,
) -> Result<RuntimeManifest> {
    super::cache_preparation::build_runtime_cache(project_root, path, identity)
}

pub(crate) fn build_and_publish_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<SceneCacheIndex> {
    Ok(build_and_publish_scene_cache_generation_inner(project_root, descriptor, false)?
        .expect("unmanaged Scene cache generation must publish"))
}
pub(crate) fn build_and_publish_managed_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<Option<SceneCacheIndex>> {
    build_and_publish_scene_cache_generation_inner(project_root, descriptor, true)
}
fn build_and_publish_scene_cache_generation_inner(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
    managed: bool,
) -> Result<Option<SceneCacheIndex>> {
    let store = SceneCacheStore::new(project_root);
    let caller_source_content_hash = descriptor.source_content_hash;
    let caller_state = descriptor.state;
    let expected_descriptor = if managed {
        let Some(expected) = store
            .load_descriptor(descriptor.scene_id)?
            .filter(|current| current.generation == descriptor.generation)
        else {
            return Ok(None);
        };
        if expected.schema_version != descriptor.schema_version
            || expected.config_hash != descriptor.config_hash
            || expected.source_stamp != descriptor.source_stamp
            || expected
                .source_content_hash
                .zip(caller_source_content_hash)
                .is_some_and(|(expected_hash, caller_hash)| expected_hash != caller_hash)
        {
            return Ok(None);
        }
        Some(expected)
    } else {
        None
    };
    let (index, spatial) =
        build_scene_cache_index(project_root, descriptor.scene_id, descriptor.generation)?;
    let payload_bytes = index
        .entries
        .iter()
        .map(|entry| {
            entry.geometry.as_ref().map_or(0, |blob| blob.byte_size)
                + entry.material.as_ref().map_or(0, |blob| blob.byte_size)
                + entry.animation.as_ref().map_or(0, |blob| blob.byte_size)
        })
        .sum();
    let mut descriptor = expected_descriptor.as_ref().unwrap_or(descriptor).clone();
    if descriptor.state == SceneCacheState::Building && caller_state != SceneCacheState::Building {
        descriptor.state = caller_state;
    }
    if descriptor.source_content_hash.is_none() {
        descriptor.source_content_hash = caller_source_content_hash;
    }
    descriptor.prim_count = index.entries.len() as u64;
    descriptor.cacheable_count = index.entries.iter().filter(|entry| entry.cacheable).count() as u64;
    descriptor.estimated_cpu_bytes = payload_bytes;
    descriptor.estimated_gpu_bytes = payload_bytes;
    if let Some(expected) = descriptor.source_content_hash {
        let target = super::cache::ProjectCacheTarget::Scene {
            id: descriptor.scene_id.to_string(),
        };
        ensure!(
            super::cache::target_content_hash(project_root, &target)? == expected,
            "Scene source changed during cache build"
        );
    }
    #[cfg(test)]
    tests::run_build_publish_hook(project_root);
    if managed {
        let expected_descriptor = expected_descriptor
            .expect("managed Scene cache publication captured its CAS snapshot");
        if store
            .publish_generation_if_current(&expected_descriptor, &descriptor, &index, &spatial)?
            .is_none()
        {
            return Ok(None);
        }
    } else {
        store.publish_generation(&descriptor, &index, &spatial)?;
    }
    publish_current_project_cache_lookup(project_root)?;
    Ok(Some(index))
}
#[cfg(test)]
#[path = "cache_warm_runtime_tests.rs"]
mod tests;
pub(crate) fn publish_project_cache_lookup(
    project_root: &Path,
    lookup: &ProjectCacheLookup,
) -> Result<Vec<u8>> {
    let payload = ProjectCacheIndexV2 {
        schema_version: PROJECT_CACHE_INDEX_SCHEMA_VERSION,
        generations: lookup.persistent_generations(),
        rows: lookup.persistent_rows(),
    };
    let bytes = serde_json::to_vec(&payload).context("encode Project cache index")?;
    let layout = crate::project::storage::ProjectStorageLayout::new(project_root);
    let path = layout.project_cache_index_path();
    if fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(bytes);
    }
    let parent = path
        .parent()
        .context("Project cache index has no parent directory")?;
    fs::create_dir_all(parent).context("create Project cache directory")?;
    let temporary = parent.join(format!(".project-index.{}.tmp", Uuid::new_v4()));
    crate::project::catalog::manifest_store::write_bytes_atomic(&temporary, &path, &bytes)
        .context("publish Project cache index")?;
    Ok(bytes)
}
pub(crate) fn publish_current_project_cache_lookup(project_root: &Path) -> Result<Vec<u8>> {
    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(project_root)?;
    let store = SceneCacheStore::new(project_root);
    let mut indexes = Vec::new();
    for scene in manifest.scenes() {
        if let Some(index) = store.load_index(scene.id)? {
            indexes.push(index);
        }
    }
    let lookup = ProjectCacheLookup::from_scene_indexes(&indexes)?;
    publish_project_cache_lookup(project_root, &lookup)
}

/// Load the already-published immutable Project lookup. This reads one compact
/// Project-index file; it deliberately does not reopen or scan every Scene
/// index during a presentation update.
pub(crate) fn load_published_project_cache_lookup(
    project_root: &Path,
) -> Result<Option<ProjectCacheLookup>> {
    let path = crate::project::storage::ProjectStorageLayout::new(project_root)
        .project_cache_index_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let persisted: ProjectCacheIndexV2 =
        serde_json::from_slice(&bytes).context("decode Project cache index")?;
    ensure!(
        persisted.schema_version == PROJECT_CACHE_INDEX_SCHEMA_VERSION,
        "unsupported Project cache index schema version {}",
        persisted.schema_version
    );
    Ok(Some(ProjectCacheLookup::from_persistent_rows(
        persisted.generations,
        persisted.rows,
    )?))
}
