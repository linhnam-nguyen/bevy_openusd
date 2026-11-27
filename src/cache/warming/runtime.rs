//! Headless Scene-owned cache building plus legacy non-Scene runtime warming.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_project::SceneId;
use uuid::Uuid;
use viewport_protocol::RuntimeManifest;

use super::super::contract::{
    CacheObjectRef, PROJECT_CACHE_INDEX_SCHEMA_VERSION, ProjectCacheLookup, SceneCacheAddress,
    SceneCacheIndex, SceneCacheState,
};
pub(crate) use super::super::scene::build::{
    assign_parent_indexes, build_scene_cache_index, build_spatial_index,
};
use super::super::{ProjectCacheIdentity, SceneCacheDescriptorV3, SceneCacheStore};

#[derive(Deserialize, Serialize)]
struct ProjectCacheIndexV2 {
    schema_version: u16,
    generations: Vec<(SceneId, u64)>,
    rows: Vec<(SceneCacheAddress, CacheObjectRef)>,
}

pub(in crate::cache) fn build_runtime_cache(
    project_root: &Path,
    path: &Path,
    identity: &ProjectCacheIdentity,
) -> Result<RuntimeManifest> {
    super::super::preparation::build_runtime_cache(project_root, path, identity)
}

trait GenerationPublishObserver {
    fn before_publish(&self, project_root: &Path);
}

struct NoopPublishObserver;

impl GenerationPublishObserver for NoopPublishObserver {
    #[inline(always)]
    fn before_publish(&self, _project_root: &Path) {}
}

pub(crate) fn build_and_publish_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<SceneCacheIndex> {
    Ok(build_and_publish_scene_cache_generation_inner(
        project_root,
        descriptor,
        false,
        None,
        &NoopPublishObserver,
    )?
    .map(|(index, _)| index)
    .context("Scene cache generation was not authorized for publication")?)
}
pub(crate) fn build_and_publish_managed_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<Option<SceneCacheIndex>> {
    build_and_publish_managed_scene_cache_generation_with_observer(
        project_root,
        descriptor,
        &NoopPublishObserver,
    )
}
pub(crate) fn build_and_publish_managed_scene_cache_generation_with_delta(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
    delta_hint: Option<&super::super::changes::SceneDeltaHint>,
) -> Result<Option<(SceneCacheIndex, super::super::scene::update::IncrementalBuildStats)>> {
    build_and_publish_scene_cache_generation_inner(
        project_root,
        descriptor,
        true,
        delta_hint,
        &NoopPublishObserver,
    )
}
fn build_and_publish_managed_scene_cache_generation_with_observer<O: GenerationPublishObserver>(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
    observer: &O,
) -> Result<Option<SceneCacheIndex>> {
    Ok(build_and_publish_scene_cache_generation_inner(
        project_root,
        descriptor,
        true,
        None,
        observer,
    )?
    .map(|(index, _)| index))
}
fn build_and_publish_scene_cache_generation_inner<O: GenerationPublishObserver>(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
    managed: bool,
    delta_hint: Option<&super::super::changes::SceneDeltaHint>,
    observer: &O,
) -> Result<Option<(SceneCacheIndex, super::super::scene::update::IncrementalBuildStats)>> {
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
    let scene_path = crate::project::scene::authoring::scene_path(project_root, descriptor.scene_id);
    let live = usd_bevy::LiveStage::open_document(&scene_path)
        .context("capture Scene source before extraction")?;
    let revision_identity = super::super::identity::scene_revision_identity_from_snapshot(
        project_root, descriptor.scene_id, viewport_protocol::RuntimeProfile::NativeMedium,
        descriptor.config_hash, &live,
    ).context("qualify captured Scene cache revision")?;
    if let Some(expected_hash) = caller_source_content_hash {
        ensure!(expected_hash == revision_identity.source_closure_digest,
            "Scene source changed before cache build");
    }
    if managed && super::super::sync::qualified_cache_root(project_root, false)?.is_none() {
        return Ok(None);
    }
    let revision_store = super::super::retention::ImmutableRevisionStore::new(project_root);
    let reused = super::super::sync::with_cache_lock(project_root, "revision", || {
        let Some(verified) = revision_store.load_verified_revision(&revision_identity)? else { return Ok(None); };
        let pin = super::super::retention::CachePinGuard::acquire(project_root,
            vec![verified.record.revision_key.clone()], verified.record.referenced_blobs.clone())?;
        let scene_objects = store.object_store(descriptor.scene_id)?;
        revision_store.materialize_blobs_to(&verified.record.referenced_blobs, &scene_objects)?;
        let mut index = verified.index;
        index.generation = descriptor.generation;
        let mut spatial = verified.spatial;
        spatial.generation = descriptor.generation;
        Ok(Some((index, spatial, pin)))
    })?;
    let (index, spatial, build_pin, build_stats) = if let Some((index, spatial, pin)) = reused {
        let stats = super::super::scene::update::IncrementalBuildStats {
            used_incremental: true,
            geometry_blobs_reused: index
                .entries
                .iter()
                .filter(|entry| entry.geometry.is_some())
                .count(),
            material_blobs_reused: index
                .entries
                .iter()
                .filter(|entry| entry.material.is_some())
                .count(),
            animation_blobs_reused: index
                .entries
                .iter()
                .filter(|entry| entry.animation.is_some())
                .count(),
            ..Default::default()
        };
        (index, spatial, pin, stats)
    } else if let Some(hint) = delta_hint.filter(|hint| hint.targets_revision(&revision_identity))
        && let Some(base_digest) = hint.base_revision_identity
    {
        // Load and pin the verified immutable base revision under the common
        // cache lock so GC cannot collect base blobs while we build incrementally.
        let base_pinned = super::super::sync::with_cache_lock(project_root, "revision", || {
            let Some(base) =
                revision_store.load_verified_revision_by_key(&base_digest.to_hex(), None)?
            else {
                return Ok(None);
            };
            let mut revision_keys = vec![revision_identity.storage_key()];
            if base.record.revision_key != revision_identity.storage_key() {
                revision_keys.push(base.record.revision_key.clone());
            }
            let pin = super::super::retention::CachePinGuard::acquire(
                project_root,
                revision_keys,
                base.record.referenced_blobs.clone(),
            )?;
            Ok(Some((base, pin)))
        })?;
        if let Some((base, pin)) = base_pinned {
            let (index, spatial, stats) =
                super::super::scene::update::build_scene_cache_index_incremental_from_snapshot(
                    project_root,
                    descriptor.scene_id,
                    descriptor.generation,
                    &live,
                    &base,
                    &revision_identity,
                    hint,
                )?;
            (index, spatial, pin, stats)
        } else {
            let pin = super::super::retention::CachePinGuard::acquire(
                project_root,
                vec![revision_identity.storage_key()],
                Vec::new(),
            )?;
            let (index, spatial) = super::super::scene::build::build_scene_cache_index_from_snapshot(
                project_root,
                descriptor.scene_id,
                descriptor.generation,
                &live,
            )?;
            let stats = super::super::scene::update::IncrementalBuildStats::full_fallback(
                super::super::changes::FullRebuildReason::MissingBaseRevision,
                &index,
            );
            (index, spatial, pin, stats)
        }
    } else {
        // Register protection before extracting. Extraction writes only to the
        // retained per-Scene pool; the shared pool is populated under the lock.
        let pin = super::super::retention::CachePinGuard::acquire(
            project_root,
            vec![revision_identity.storage_key()],
            Vec::new(),
        )?;
        let (index, spatial) = super::super::scene::build::build_scene_cache_index_from_snapshot(
            project_root,
            descriptor.scene_id,
            descriptor.generation,
            &live,
        )?;
        let reason = if delta_hint.is_some() {
            super::super::changes::FullRebuildReason::UnqualifiedChangeChain
        } else {
            super::super::changes::FullRebuildReason::MissingBaseRevision
        };
        let stats = super::super::scene::update::IncrementalBuildStats::full_fallback(reason, &index);
        (index, spatial, pin, stats)
    };
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
    descriptor.revision_identity = Some(revision_identity.digest());
    if descriptor.source_content_hash.is_none() {
        descriptor.source_content_hash = Some(revision_identity.source_closure_digest);
    }
    descriptor.prim_count = index.entries.len() as u64;
    descriptor.cacheable_count =
        index.entries.iter().filter(|entry| entry.cacheable).count() as u64;
    descriptor.estimated_cpu_bytes = payload_bytes;
    descriptor.estimated_gpu_bytes = payload_bytes;
    if let Some(expected) = descriptor.source_content_hash {
        let target = super::super::ProjectCacheTarget::Scene {
            id: descriptor.scene_id.to_string(),
        };
        ensure!(
            crate::project::assets::closure::target_content_hash(project_root, &target)? == expected,
            "Scene source changed during cache build"
        );
    }
    // The test/production observer runs outside every cache transaction.
    observer.before_publish(project_root);
    let committed = super::super::sync::with_cache_lock(project_root, "publish", || {
        let target = super::super::ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        if super::super::sync::TombstoneStore::new(project_root).is_retired_at(&target, descriptor.generation)? {
            return Ok(false);
        }
        if managed && store.load_descriptor(descriptor.scene_id)?.as_ref() != expected_descriptor.as_ref() {
            return Ok(false);
        }
        let pending = super::super::sync::PendingIntentStore::new(project_root);
        // The unmanaged entry point cannot bypass managed Ready authorization.
        let requires_desired = managed
            || (descriptor.state == SceneCacheState::Ready && store.has_managed_manifest()?);
        if requires_desired && !store.matches_desired_revision(&descriptor)? {
            return Ok(false);
        }
        let latest = super::super::identity::compute_scene_revision_identity(project_root,
            descriptor.scene_id, viewport_protocol::RuntimeProfile::NativeMedium, descriptor.config_hash)?;
        ensure!(latest == revision_identity, "Scene source or composition changed during cache build");
        let scene_objects = store.object_store(descriptor.scene_id)?;
        revision_store.store_verified_revision_for_build(&revision_identity, &descriptor, &index, &spatial, &scene_objects, &build_pin)?;
        if let Some(expected) = expected_descriptor.as_ref() {
            if store.publish_generation_if_current(expected, &descriptor, &index, &spatial)?.is_none() {
                return Ok(false);
            }
        } else {
            store.publish_generation(&descriptor, &index, &spatial)?;
        }
        pending.acknowledge_if_generation(&target, descriptor.generation, descriptor.revision_identity)?;
        Ok(true)
    })?;
    drop(build_pin);
    if !committed { return Ok(None); }
    publish_current_project_cache_lookup(project_root)?;
    if managed {
        let source = crate::project::scene::authoring::scene_path(project_root, descriptor.scene_id);
        super::super::preparation::prepare_document_assembly(project_root, &source);
    }
    Ok(Some((index, build_stats)))
}
#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/runtime_tests.rs"]
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
    if super::super::sync::qualify_cache_file(&path, false)?
        && fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(bytes);
    }
    let parent = path
        .parent()
        .context("Project cache index has no parent directory")?;
    super::super::sync::qualify_cache_directory(parent, true)?;
    let temporary = parent.join(format!(".project-index.{}.tmp", Uuid::new_v4()));
    super::super::sync::write_bytes_atomic_durable(&temporary, &path, &bytes)
        .context("publish Project cache index")?;
    Ok(bytes)
}
pub(crate) fn publish_current_project_cache_lookup(project_root: &Path) -> Result<Vec<u8>> {
    super::super::sync::with_cache_lock(project_root, "lookup", || {
        let lookup = qualified_project_lookup(project_root)?;
        publish_project_cache_lookup(project_root, &lookup)
    })
}

fn qualified_project_lookup(project_root: &Path) -> Result<ProjectCacheLookup> {
    let manifest = crate::project::catalog::manifest_store::ManifestStore::read_validated(project_root)?;
    let store = SceneCacheStore::new(project_root);
    let mut indexes = Vec::new();
    for scene in manifest.scenes() {
        if let Some(activation) = store.load_activation_for_repair(scene.id)? {
            indexes.push(activation.index);
        }
    }
    ProjectCacheLookup::from_scene_indexes(&indexes)
}

/// Verify persisted lookup metadata against current qualified Scene identities.
/// Missing resources may still be repaired by the dedicated demand owner.
pub(crate) fn load_published_project_cache_lookup(
    project_root: &Path,
) -> Result<Option<ProjectCacheLookup>> {
    if super::super::sync::cache_invalidation_barrier(project_root)? { return Ok(None); }
    let path =
        crate::project::storage::ProjectStorageLayout::new(project_root).project_cache_index_path();
    if !super::super::sync::qualify_cache_file(&path, false)? { return Ok(None); }
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
    let expected = qualified_project_lookup(project_root)?;
    if persisted.generations != expected.persistent_generations()
        || persisted.rows != expected.persistent_rows() {
        return Ok(None);
    }
    Ok(Some(ProjectCacheLookup::from_persistent_rows(persisted.generations, persisted.rows)?))
}
