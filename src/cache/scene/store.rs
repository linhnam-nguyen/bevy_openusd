//! Scene-owned V3 cache storage and Scene-keyed generation serialization.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use usd_model::HashDigest;
use usd_project::SceneId;
use uuid::Uuid;

use super::super::blobs::FilesystemBlobStore;
use super::super::contract::{
    ProjectCacheTarget, SceneCacheActivation, SceneCacheDescriptorV3, SceneCacheIndex,
    SceneCacheState, SceneSpatialIndex,
};
use super::super::retention::{CachePinGuard, ImmutableRevisionStore};
use super::super::sync::{
    CacheLifecycleState, PendingIntentStore, TombstoneStore, with_cache_lock,
    write_bytes_atomic_durable,
};
use crate::project::storage::ProjectStorageLayout;

#[derive(Clone, Debug)]
pub(crate) struct SceneCacheStore {
    layout: ProjectStorageLayout,
}

impl SceneCacheStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            layout: ProjectStorageLayout::new(project_root),
        }
    }

    pub(crate) fn object_store(&self, scene_id: SceneId) -> Result<FilesystemBlobStore> {
        FilesystemBlobStore::new(self.layout.scene_cache_objects_dir(scene_id))
    }

    pub(crate) fn shared_object_store(&self) -> Result<FilesystemBlobStore> {
        FilesystemBlobStore::new(self.layout.shared_cache_objects_dir())
    }

    fn lock_scope(scene_id: SceneId) -> String {
        format!("scene-{scene_id}")
    }

    pub(crate) fn with_generation_lock<T>(
        &self,
        scene_id: SceneId,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        with_cache_lock(self.layout.root(), &Self::lock_scope(scene_id), operation)
    }

    pub(crate) fn advance_generation(
        &self,
        scene_id: SceneId,
        config_hash: HashDigest,
    ) -> Result<u64> {
        self.with_generation_lock(scene_id, || {
            let target = ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            };
            let tombstones = TombstoneStore::new(self.layout.root());
            let retired_floor = tombstones.reactivation_floor(&target)?;
            if self.has_managed_manifest()? {
                let manifest =
                    crate::project::catalog::manifest_store::ManifestStore::read_validated(
                        self.layout.root(),
                    )?;
                ensure!(
                    manifest.scene(scene_id).is_some(),
                    "cannot reactivate an unregistered Scene"
                );
            } else {
                ensure!(
                    retired_floor == 0,
                    "cannot reactivate a retired Scene without a validated Project manifest"
                );
            }
            let pending = PendingIntentStore::new(self.layout.root());
            let desired_floor = pending.generation_floor(&target)?;
            let descriptor_floor = match self.load_descriptor(scene_id) {
                Ok(Some(descriptor)) => descriptor.generation,
                Ok(None) => 0,
                Err(_) if desired_floor > 0 => desired_floor,
                Err(error) => return Err(error).context("cannot establish Scene generation floor"),
            };
            let generation = descriptor_floor
                .max(desired_floor)
                .max(retired_floor)
                .checked_add(1)
                .context("Scene cache generation overflow")?;
            let identity = super::super::identity::compute_scene_revision_identity(
                self.layout.root(), scene_id, viewport_protocol::RuntimeProfile::NativeMedium, config_hash);
            let mut intent = super::super::sync::PendingCacheIntent::new(
                target, generation, identity.as_ref().ok().map(|value| value.source_closure_digest), config_hash);
            intent.desired_revision_identity = identity.as_ref().ok().map(|value| value.digest());
            // Persist first. If descriptor publication is interrupted, recovery
            // can reconstruct the Building pointer from this newer intent.
            pending.record(&intent)?;
            self.publish_descriptor(&SceneCacheDescriptorV3::invalidated(scene_id, generation, config_hash))?;
            Ok(generation)
        })
    }

    /// Publish a Building descriptor when the normal generation read is
    /// available and the next generation is representable. Read and overflow
    /// errors are returned so recovery can remove the Scene cache instead of
    /// manufacturing a non-monotonic boundary.
    pub(crate) fn force_invalidate_generation(
        &self,
        scene_id: SceneId,
        config_hash: HashDigest,
    ) -> Result<()> {
        let _ = self.advance_generation(scene_id, config_hash)?;
        Ok(())
    }

    /// Retire a deleted Scene target atomically under its cross-process lock:
    /// records a durable tombstone strictly above any current generation,
    /// removes pending preparation intents, and removes the mutable current
    /// Scene cache pointer while leaving shared immutable blobs for GC.
    pub(crate) fn retire_scene(&self, scene_id: SceneId) -> Result<u64> {
        if super::super::sync::qualified_cache_root(self.layout.root(), false)?.is_none() {
            return Ok(0);
        }
        self.with_generation_lock(scene_id, || {
            let target = ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            };
            let tombstones = TombstoneStore::new(self.layout.root());
            let previous_tombstone = tombstones.reactivation_floor(&target)?;
            let pending = PendingIntentStore::new(self.layout.root());
            let desired_floor = pending.generation_floor(&target)?;
            let current_generation = match self.load_descriptor(scene_id) {
                Ok(Some(descriptor)) => descriptor.generation,
                Ok(None) | Err(_) => 0,
            };
            let retired_generation = current_generation
                .max(desired_floor)
                .max(previous_tombstone)
                .saturating_add(1);
            tombstones.retire(&target, retired_generation)?;
            pending.remove(&target)?;
            let root = super::super::sync::qualified_cache_root(self.layout.root(), false)?
                .context("missing cache root")?;
            // Legacy readers may hold resource identifiers without a lease.
            // Retirement disables the pointer, never deletes their objects or indexes.
            let path = self.layout.scene_cache_descriptor_path(scene_id);
            if super::super::sync::qualify_cache_file(&path, false)? {
                super::super::gc::safe_remove_cache_file(&root, &path)?;
            }
            Ok(retired_generation)
        })
    }

    /// Inspect the canonical six-state lifecycle of a Scene cache target
    /// against an optional expected strong source digest and config hash.
    pub(crate) fn lifecycle_state(
        &self,
        scene_id: SceneId,
        expected_source_hash: Option<HashDigest>,
        expected_config_hash: Option<HashDigest>,
    ) -> Result<CacheLifecycleState> {
        let target = ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        };
        let tombstones = TombstoneStore::new(self.layout.root());
        let tombstone = tombstones.load(&target)?;
        let descriptor = match self.load_descriptor(scene_id) {
            Ok(Some(descriptor)) => descriptor,
            Ok(None) => {
                return Ok(if tombstone.is_some() {
                    CacheLifecycleState::Retired
                } else {
                    CacheLifecycleState::Missing
                });
            }
            Err(_) => return Ok(CacheLifecycleState::Failed),
        };
        if descriptor.state == SceneCacheState::Retired
            || tombstone
                .as_ref()
                .is_some_and(|t| descriptor.generation <= t.retired_generation)
        {
            return Ok(CacheLifecycleState::Retired);
        }
        match descriptor.state {
            SceneCacheState::Empty => Ok(CacheLifecycleState::Missing),
            SceneCacheState::Building => Ok(CacheLifecycleState::Building),
            SceneCacheState::Stale => Ok(CacheLifecycleState::Stale),
            SceneCacheState::Retired => Ok(CacheLifecycleState::Retired),
            SceneCacheState::Failed | SceneCacheState::FallbackRequired => {
                Ok(CacheLifecycleState::Failed)
            }
            SceneCacheState::Partial | SceneCacheState::Ready => {
                if let Some(expected_config) = expected_config_hash
                    && descriptor.config_hash != expected_config
                {
                    return Ok(CacheLifecycleState::Stale);
                }
                if let Some(expected_source) = expected_source_hash
                    && descriptor.source_content_hash != Some(expected_source)
                {
                    return Ok(CacheLifecycleState::Stale);
                }
                let Ok(Some(index)) = self.load_index(scene_id) else {
                    return Ok(CacheLifecycleState::Failed);
                };
                if super::super::sync::cache_invalidation_barrier(self.layout.root())?
                    || !self.matches_current_revision(&descriptor).unwrap_or(false) {
                    return Ok(CacheLifecycleState::Stale);
                }
                if self.verify_index_blobs(scene_id, &index).is_err()
                    || !matches!(self.load_spatial(scene_id), Ok(Some(_))) {
                    return Ok(CacheLifecycleState::Failed);
                }
                Ok(if descriptor.state == SceneCacheState::Ready { CacheLifecycleState::Ready }
                    else { CacheLifecycleState::Stale })
            }
        }
    }

    /// A legacy manifest also denotes a managed Project. Only confirmed
    /// absence permits the unmanaged compatibility path.
    pub(in crate::cache) fn has_managed_manifest(&self) -> Result<bool> {
        let path = self.layout.readable_manifest_path();
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| {
                format!("inspect Project manifest {}", path.display())
            }),
        }
    }

    /// The caller holds the Project transaction for publication. An
    /// acknowledged record remains authoritative; absence never authorizes
    /// a managed Ready generation.
    pub(in crate::cache) fn matches_desired_revision(
        &self,
        descriptor: &SceneCacheDescriptorV3,
    ) -> Result<bool> {
        let target = ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        if TombstoneStore::new(self.layout.root()).is_retired_at(&target, descriptor.generation)? {
            return Ok(false);
        }
        let Some(desired) = PendingIntentStore::new(self.layout.root()).load_record(&target)? else {
            return Ok(false);
        };
        Ok(desired.generation == descriptor.generation
            && desired.config_hash == descriptor.config_hash
            && descriptor.revision_identity.is_some()
            && descriptor.source_content_hash.is_some()
            && desired.desired_revision_identity == descriptor.revision_identity
            && desired.desired_source_hash == descriptor.source_content_hash)
    }

    /// Compare the full identity, not merely the mutable generation or owned
    /// source hash. An unknown identity cannot qualify a payload projection.
    fn matches_current_revision(&self, descriptor: &SceneCacheDescriptorV3) -> Result<bool> {
        let target = ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        if TombstoneStore::new(self.layout.root()).is_retired_at(&target, descriptor.generation)? {
            return Ok(false);
        }
        let managed = self.has_managed_manifest()?;
        if managed && descriptor.state == SceneCacheState::Ready {
            if !self.matches_desired_revision(descriptor)? {
                return Ok(false);
            }
        } else if let Some(desired) = PendingIntentStore::new(self.layout.root()).load_record(&target)? {
            if desired.generation != descriptor.generation
                || desired.desired_revision_identity != descriptor.revision_identity
                || desired.desired_source_hash != descriptor.source_content_hash
                || desired.config_hash != descriptor.config_hash {
                return Ok(false);
            }
        }
        if descriptor.state == SceneCacheState::Partial
            && descriptor.revision_identity.is_none()
            && !managed
        {
            return Ok(true);
        }
        let actual = super::super::identity::compute_scene_revision_identity(self.layout.root(),
            descriptor.scene_id, viewport_protocol::RuntimeProfile::NativeMedium, descriptor.config_hash)?;
        Ok(descriptor.revision_identity == Some(actual.digest())
            && descriptor.source_content_hash == Some(actual.source_closure_digest))
    }

    /// Verify that every blob referenced by `index` (geometry, material,
    /// animation, and transitive material textures) is present and digest-valid
    /// in the Scene object store.
    pub(crate) fn verify_index_blobs(
        &self,
        scene_id: SceneId,
        index: &SceneCacheIndex,
    ) -> Result<()> {
        let object_store = self.object_store(scene_id)?;
        let referenced = ImmutableRevisionStore::collect_referenced_blobs(&object_store, index)?;
        ImmutableRevisionStore::verify_blobs_present(&object_store, &referenced)
    }

    /// Remove all disposable cache state when the authoritative manifest is
    /// unavailable and no complete Scene membership list can be derived.
    pub(crate) fn remove_all_derived_cache(&self) -> Result<()> {
        // An unavailable graph cannot prove that any payload is disposable.
        super::super::sync::set_cache_invalidation_barrier(self.layout.root())
    }

    pub(crate) fn publish_descriptor(
        &self,
        descriptor: &SceneCacheDescriptorV3,
    ) -> Result<PathBuf> {
        self.with_generation_lock(descriptor.scene_id, || {
            ensure!(
                descriptor.state != SceneCacheState::Ready || !self.has_managed_manifest()?,
                "managed Ready requires verified generation publication"
            );
            self.write_descriptor_locked(descriptor)
        })
    }

    /// Used only under the Project transaction. A Ready caller must first
    /// validate the generation, full revision and complete artifact closure.
    fn write_descriptor_locked(&self, descriptor: &SceneCacheDescriptorV3) -> Result<PathBuf> {
        descriptor.validate()?;
        let target = ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        ensure!(!TombstoneStore::new(self.layout.root()).is_retired_at(&target, descriptor.generation)?,
            "refusing descriptor at or below retired generation");
        ensure!(PendingIntentStore::new(self.layout.root()).generation_floor(&target)? <= descriptor.generation,
            "refusing descriptor below durable desired generation");
        let path = self.layout.scene_cache_descriptor_path(descriptor.scene_id);
        let bytes = serde_json::to_vec_pretty(descriptor).context("encode Scene cache descriptor")?;
        self.publish_scene_bytes(descriptor.scene_id, "descriptor", &path, &bytes)?;
        Ok(path)
    }

    pub(crate) fn publish_generation(
        &self,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
    ) -> Result<SceneCacheDescriptorV3> {
        self.with_generation_lock(descriptor.scene_id, || self.publish_generation_locked(descriptor, index, spatial))
    }

    fn publish_generation_locked(&self, descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex, spatial: &SceneSpatialIndex) -> Result<SceneCacheDescriptorV3> {
        index.validate(descriptor.scene_id, descriptor.generation)?;
        spatial.validate(descriptor.scene_id, descriptor.generation)?;
        let target = ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        ensure!(!TombstoneStore::new(self.layout.root()).is_retired_at(&target, descriptor.generation)?,
            "refusing publication at retired generation");
        ensure!(PendingIntentStore::new(self.layout.root()).generation_floor(&target)? <= descriptor.generation,
            "refusing publication below durable generation");
        if descriptor.state == SceneCacheState::Ready || descriptor.revision_identity.is_some() {
            ensure!(self.matches_current_revision(descriptor)?, "incompatible or unknown Scene revision");
        }
        if matches!(
            descriptor.state,
            SceneCacheState::Ready | SceneCacheState::Partial
        ) {
            self.verify_index_blobs(descriptor.scene_id, index)
                .context("refusing to publish Scene cache pointer with missing or corrupt blobs")?;
        }
        let index_bytes = serde_json::to_vec(index).context("encode Scene cache index")?;
        let spatial_bytes = serde_json::to_vec(spatial).context("encode Scene spatial index")?;
        let index_digest = HashDigest::new(*blake3::hash(&index_bytes).as_bytes());
        let spatial_digest = HashDigest::new(*blake3::hash(&spatial_bytes).as_bytes());

        let index_path = self.layout.scene_cache_index_path(descriptor.scene_id);
        self.publish_scene_bytes(descriptor.scene_id, "index", &index_path, &index_bytes)?;
        let spatial_path = self.layout.scene_cache_spatial_path(descriptor.scene_id);
        self.publish_scene_bytes(
            descriptor.scene_id,
            "spatial",
            &spatial_path,
            &spatial_bytes,
        )?;

        let mut published = descriptor.clone();
        published.index_digest = index_digest;
        published.spatial_digest = Some(spatial_digest);
        self.write_descriptor_locked(&published)?;
        Ok(published)
    }

    pub(crate) fn publish_generation_if_current(
        &self,
        expected: &SceneCacheDescriptorV3,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
    ) -> Result<Option<SceneCacheDescriptorV3>> {
        if super::super::sync::qualified_cache_root(self.layout.root(), false)?.is_none() {
            return Ok(None);
        }
        self.with_generation_lock(expected.scene_id, || {
            let target = ProjectCacheTarget::Scene {
                id: expected.scene_id.to_string(),
            };
            if TombstoneStore::new(self.layout.root())
                .is_retired_at(&target, descriptor.generation)?
            {
                return Ok(None);
            }
            if self.load_descriptor(expected.scene_id)?.as_ref() != Some(expected) {
                return Ok(None);
            }
            self.publish_generation(descriptor, index, spatial)
                .map(Some)
        })
    }

    pub(crate) fn remove_scene(&self, scene_id: SceneId) -> Result<()> {
        let _ = self.retire_scene(scene_id)?;
        Ok(())
    }

    /// Load a verified activation view together with an RAII [`CachePinGuard`]
    /// protecting its revision and referenced blobs from concurrent GC.
    pub(crate) fn load_activation_pinned(
        &self,
        scene_id: SceneId,
    ) -> Result<Option<(SceneCacheActivation, CachePinGuard)>> {
        if super::super::sync::qualified_cache_root(self.layout.root(), false)?.is_none() {
            return Ok(None);
        }
        self.with_generation_lock(scene_id, || self.load_activation_pinned_locked(scene_id))
    }

    fn load_activation_pinned_locked(&self, scene_id: SceneId) -> Result<Option<(SceneCacheActivation, CachePinGuard)>> {
        let Some(activation) = self.load_activation(scene_id)? else {
            return Ok(None);
        };
        let object_store = self.object_store(scene_id)?;
        let referenced =
            match ImmutableRevisionStore::collect_referenced_blobs(&object_store, &activation.index)
            {
                Ok(referenced) => referenced,
                Err(_) => return Ok(None),
            };
        let revision_keys = activation
            .descriptor
            .revision_identity
            .map(|digest| vec![digest.to_hex()])
            .unwrap_or_default();
        let pin = CachePinGuard::acquire(self.layout.root(), revision_keys, referenced.clone())?;
        if ImmutableRevisionStore::verify_blobs_present(&object_store, &referenced).is_err() {
            return Ok(None);
        }
        // Re-verify pointer did not advance while pinning.
        let Some(current) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        if current.generation != activation.descriptor.generation
            || current.index_digest != activation.descriptor.index_digest
        {
            return Ok(None);
        }
        Ok(Some((activation, pin)))
    }

    pub(crate) fn load_index(&self, scene_id: SceneId) -> Result<Option<SceneCacheIndex>> {
        let Some(descriptor) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        let path = self.layout.scene_cache_index_path(scene_id);
        let Some(bytes) = self.read_generation_bytes(&path, descriptor.index_digest)? else {
            return Ok(None);
        };
        let index: SceneCacheIndex = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene cache index {}", path.display()))?;
        index.validate(scene_id, descriptor.generation)?;
        Ok(Some(index))
    }

    /// Load the stable metadata-first activation view by SceneId. The
    /// descriptor is checked before and after the index read so a generation
    /// rollover cannot publish a mixed descriptor/index pair.
    pub(crate) fn load_activation(
        &self,
        scene_id: SceneId,
    ) -> Result<Option<SceneCacheActivation>> {
        let Some(activation) = self.load_activation_for_repair(scene_id)? else { return Ok(None); };
        if self.verify_index_blobs(scene_id, &activation.index).is_err() { return Ok(None); }
        Ok(Some(activation))
    }

    /// Qualified metadata remains available to the exact-path repair owner
    /// when a payload is missing; this is not a Ready resource lease.
    pub(crate) fn load_activation_for_repair(&self, scene_id: SceneId) -> Result<Option<SceneCacheActivation>> {
        let Some(descriptor) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        if !matches!(
            descriptor.state,
            SceneCacheState::Partial | SceneCacheState::Ready
        ) {
            return Ok(None);
        }
        if super::super::sync::cache_invalidation_barrier(self.layout.root())?
            || !self.matches_current_revision(&descriptor).unwrap_or(false) {
            return Ok(None);
        }
        let Some(index) = self.load_index(scene_id)? else {
            return Ok(None);
        };
        if !matches!(self.load_spatial(scene_id), Ok(Some(_))) {
            return Ok(None);
        }
        let Some(current) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        if current != descriptor
            || !matches!(
                current.state,
                SceneCacheState::Partial | SceneCacheState::Ready
            )
        {
            return Ok(None);
        }
        Ok(Some(SceneCacheActivation {
            descriptor: current,
            index,
        }))
    }

    pub(crate) fn load_spatial(&self, scene_id: SceneId) -> Result<Option<SceneSpatialIndex>> {
        let Some(descriptor) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        let Some(expected) = descriptor.spatial_digest else {
            return Ok(None);
        };
        let path = self.layout.scene_cache_spatial_path(scene_id);
        let Some(bytes) = self.read_generation_bytes(&path, expected)? else {
            return Ok(None);
        };
        let spatial: SceneSpatialIndex = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene spatial index {}", path.display()))?;
        spatial.validate(scene_id, descriptor.generation)?;
        Ok(Some(spatial))
    }

    pub(crate) fn load_descriptor(
        &self,
        scene_id: SceneId,
    ) -> Result<Option<SceneCacheDescriptorV3>> {
        let Some(root) = super::super::sync::qualified_cache_root(self.layout.root(), false)? else { return Ok(None); };
        let path = self.layout.scene_cache_descriptor_path(scene_id);
        match fs::symlink_metadata(&path) {
            Ok(_) => { super::super::sync::qualify_cache_entry(&root, &path)?; },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect Scene descriptor path"),
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let descriptor: SceneCacheDescriptorV3 = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene cache descriptor {}", path.display()))?;
        ensure!(
            descriptor.scene_id == scene_id,
            "Scene cache descriptor SceneId does not match lookup"
        );
        descriptor.validate()?;
        Ok(Some(descriptor))
    }

    fn publish_scene_bytes(
        &self,
        scene_id: SceneId,
        label: &str,
        path: &Path,
        bytes: &[u8],
    ) -> Result<()> {
        let parent = path
            .parent()
            .context("Scene cache artifact has no parent directory")?;
        super::super::sync::qualify_cache_directory(parent, true)?;
        let root = super::super::sync::qualified_cache_root(self.layout.root(), false)?.context("missing cache root")?;
        super::super::sync::qualify_cache_entry(&root, parent)?;
        if fs::symlink_metadata(path).is_ok() { super::super::sync::qualify_cache_entry(&root, path)?; }
        let temporary = parent.join(format!(".{label}.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic_durable(&temporary, path, bytes)
            .with_context(|| format!("publish Scene {label} for {scene_id}"))
    }

    fn read_generation_bytes(&self, path: &Path, expected: HashDigest) -> Result<Option<Vec<u8>>> {
        if !super::super::sync::qualify_cache_file(path, false)? { return Ok(None); }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let digest = HashDigest::new(*blake3::hash(&bytes).as_bytes());
        Ok((digest == expected).then_some(bytes))
    }
}

fn remove_directory_if_present(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove derived cache directory {}", path.display()))
        }
    }
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove derived cache file {}", path.display()))
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/scene/store_tests.rs"]
mod tests;
