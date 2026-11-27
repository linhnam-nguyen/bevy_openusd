//! Atomic, target-aware Project runtime-cache descriptors.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_model::HashDigest;
use uuid::Uuid;
use viewport_protocol::{RuntimeManifest, RuntimeProfile};

use super::contract::ProjectCacheTarget;
use crate::project::assets::closure::target_content_hash;
use crate::project::storage::ProjectStorageLayout;
use super::generic::write_bytes_atomic;
use super::blobs::{BlobStore, FilesystemBlobStore};
use super::sync::{PendingCacheIntent, PendingIntentStore, TombstoneStore, with_cache_lock, qualify_cache_directory, qualify_cache_file};

pub(crate) const PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION: u16 = 2;
const DESCRIPTORS_DIRECTORY: &str = "descriptors";

/// All source and runtime choices that must agree before a descriptor is
/// reusable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectCacheIdentity {
    pub(crate) target: ProjectCacheTarget,
    pub(crate) target_content_hash: HashDigest,
    pub(crate) profile: RuntimeProfile,
    pub(crate) config_hash: HashDigest,
}

impl ProjectCacheIdentity {
    pub(crate) fn for_project(
        project_root: &Path,
        target: ProjectCacheTarget,
        profile: RuntimeProfile,
        config_hash: HashDigest,
    ) -> Result<Self> {
        let target_content_hash = target_content_hash(project_root, &target)?;
        Ok(Self {
            target: target.clone(),
            target_content_hash,
            profile,
            config_hash,
        })
    }

    /// A generation binds the full composed source and recipe, not only the
    /// historical owned-source lookup hash.
    pub(crate) fn revision_digest(&self, project_root: &Path) -> Result<HashDigest> {
        let path = super::warming::target_stage_path(project_root, &self.target)?;
        let live = path.as_deref().map(usd_bevy::LiveStage::open_document).transpose()?;
        self.revision_digest_from_snapshot(project_root, live.as_ref())
    }

    /// Bind the managed target identity to the exact sealed source used to extract it.
    pub(crate) fn revision_digest_from_snapshot(&self, project_root: &Path, live: Option<&usd_bevy::LiveStage>) -> Result<HashDigest> {
        ensure!(target_content_hash(project_root, &self.target)? == self.target_content_hash,
            "Project cache source identity changed");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"nemeton.managed-project-revision.v1\0");
        hasher.update(&serde_json::to_vec(self)?);
        hasher.update(super::identity::current_cache_schema_digest().as_bytes());
        hasher.update(&PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION.to_le_bytes());
        match (super::warming::target_stage_path(project_root, &self.target)?, live) {
            (Some(path), Some(live)) => {
                let complete = super::generic::source_identity_from_opened_stage(live)?;
                ensure!(complete.entry_path == path, "captured USD source is not the current managed target");
                hasher.update(&[1]);
                hasher.update(complete.closure_digest.as_bytes());
                hasher.update(complete.resolver_digest.as_bytes());
            }
            (None, None) => { hasher.update(&[0]); }
            _ => anyhow::bail!("managed target source changed while capturing its revision"),
        }
        Ok(HashDigest::new(*hasher.finalize().as_bytes()))
    }
}

/// Descriptor state exposed internally to the Project service and, later,
/// reduced to a small cache status in the UI protocol.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectCacheState {
    Empty,
    Building,
    Ready,
    Partial,
    Stale,
    Failed,
    Retired,
    FallbackRequired,
}

/// One atomically published cache index record. The descriptor is published
/// after its referenced blobs are complete; it never owns authoritative USD.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectCacheDescriptor {
    pub(crate) schema_version: u16,
    pub(crate) identity: ProjectCacheIdentity,
    pub(crate) state: ProjectCacheState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) runtime: Option<RuntimeManifest>,
}

impl ProjectCacheDescriptor {
    pub(crate) fn new(
        identity: ProjectCacheIdentity,
        state: ProjectCacheState,
        runtime: Option<RuntimeManifest>,
    ) -> Result<Self> {
        let descriptor = Self {
            schema_version: PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            identity,
            state,
            runtime,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            "unsupported Project cache descriptor schema version {}",
            self.schema_version
        );
        if let Some(runtime) = &self.runtime {
            runtime
                .validate()
                .map_err(|error| anyhow::anyhow!("invalid cached runtime manifest: {error}"))?;
        }
        ensure!(
            self.state != ProjectCacheState::Ready || self.runtime.is_some(),
            "ready Project cache descriptor must include a runtime manifest"
        );
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct PublishedProjectCache {
    #[serde(flatten)]
    descriptor: ProjectCacheDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision_identity: Option<HashDigest>,
}

/// Filesystem owner for cache descriptors under .usdhub/cache.
#[derive(Clone, Debug)]
pub(crate) struct ProjectCacheStore {
    project_root: PathBuf,
    descriptors: PathBuf,
}

impl ProjectCacheStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            descriptors: ProjectStorageLayout::new(project_root)
                .cache_dir()
                .join(DESCRIPTORS_DIRECTORY),
        }
    }

    pub(crate) fn descriptor_path(&self, identity: &ProjectCacheIdentity) -> Result<PathBuf> {
        let bytes = serde_json::to_vec(identity).context("encode Project cache identity")?;
        let key = blake3::hash(&bytes).to_hex().to_string();
        Ok(self.descriptors.join(format!("{}.json", key)))
    }

    fn has_manifest(&self) -> bool {
        let path = ProjectStorageLayout::new(&self.project_root).readable_manifest_path();
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            _ => true,
        }
    }

    /// Prepare a managed non-Scene desired generation before queue admission.
    pub(crate) fn advance_generation(&self, target: &ProjectCacheTarget, config_hash: HashDigest) -> Result<u64> {
        with_cache_lock(&self.project_root, "desired", || {
            // Resolution checks membership even if the source cannot be hashed.
            let manifest = crate::project::catalog::manifest_store::ManifestStore::read_validated(&self.project_root)?;
            ensure!(matches!(target, ProjectCacheTarget::ProjectRoot)
                || super::warming::project_cache_targets(&manifest).contains(target), "unregistered managed cache target");
            let pending = PendingIntentStore::new(&self.project_root);
            let floor = TombstoneStore::new(&self.project_root).reactivation_floor(target)?;
            let generation = pending.generation_floor(target)?.max(floor).checked_add(1)
                .context("Project cache generation overflow")?;
            let qualified = ProjectCacheIdentity::for_project(&self.project_root, target.clone(), RuntimeProfile::NativeMedium, config_hash)
                .and_then(|identity| Ok((identity.target_content_hash, identity.revision_digest(&self.project_root)?)));
            let mut intent = PendingCacheIntent::new(target.clone(), generation, qualified.as_ref().ok().map(|value| value.0), config_hash);
            intent.desired_revision_identity = qualified.as_ref().ok().map(|value| value.1);
            pending.record(&intent)?;
            if let Err(error) = qualified { log::warn!("managed cache target identity unavailable: {error:#}"); }
            Ok(generation)
        })
    }

    /// Obtain an authorized desired generation matching `expected_revision`
    /// without superseding a newer or conflicting obligation. Reuses an
    /// existing unacknowledged matching intent, or establishes generation 1
    /// when no prior intent or retirement floor exists for the registered
    /// target. Returns `Ok(None)` when the target is retired, already
    /// acknowledged, or has a different desired revision/config.
    pub(crate) fn ensure_matching_generation(
        &self,
        identity: &ProjectCacheIdentity,
        expected_revision: HashDigest,
    ) -> Result<Option<u64>> {
        with_cache_lock(&self.project_root, "desired", || {
            let manifest =
                crate::project::catalog::manifest_store::ManifestStore::read_validated(
                    &self.project_root,
                )?;
            if !matches!(identity.target, ProjectCacheTarget::ProjectRoot)
                && !super::warming::project_cache_targets(&manifest).contains(&identity.target)
            {
                return Ok(None);
            }
            let pending = PendingIntentStore::new(&self.project_root);
            let floor = TombstoneStore::new(&self.project_root)
                .reactivation_floor(&identity.target)?;
            if let Some(record) = pending.load_record(&identity.target)? {
                if record.generation <= floor || !record.pending {
                    return Ok(None);
                }
                if record.config_hash == identity.config_hash
                    && record.desired_source_hash == Some(identity.target_content_hash)
                    && record.desired_revision_identity == Some(expected_revision)
                {
                    return Ok(Some(record.generation));
                }
                return Ok(None);
            }
            if floor > 0 {
                return Ok(None);
            }
            if identity.revision_digest(&self.project_root)? != expected_revision {
                return Ok(None);
            }
            let mut intent = PendingCacheIntent::new(
                identity.target.clone(),
                1,
                Some(identity.target_content_hash),
                identity.config_hash,
            );
            intent.desired_revision_identity = Some(expected_revision);
            pending.record(&intent)?;
            Ok(Some(1))
        })
    }

    /// Check the exact durable desired identity and retirement floor. A caller
    /// publishing a generation must hold the common Project transaction.
    fn matches_desired(&self, identity: &ProjectCacheIdentity, generation: u64, revision: HashDigest) -> Result<bool> {
        if TombstoneStore::new(&self.project_root).is_retired_at(&identity.target, generation)? { return Ok(false); }
        let Some(desired) = PendingIntentStore::new(&self.project_root).load_record(&identity.target)? else { return Ok(false); };
        if desired.generation != generation || desired.config_hash != identity.config_hash
            || desired.desired_source_hash != Some(identity.target_content_hash)
            || desired.desired_revision_identity != Some(revision) { return Ok(false); }
        Ok(identity.revision_digest(&self.project_root)? == revision)
    }

    fn verify_runtime(&self, descriptor: &ProjectCacheDescriptor) -> Result<()> {
        if let Some(runtime) = &descriptor.runtime {
            ensure!(runtime.profile == descriptor.identity.profile, "Project runtime profile mismatch");
            let objects = FilesystemBlobStore::new(ProjectStorageLayout::new(&self.project_root).cache_objects_dir())?;
            for reference in std::iter::once(&runtime.hierarchy).chain(&runtime.meshes).chain(&runtime.materials).chain(&runtime.textures) {
                let size = usize::try_from(reference.byte_size).context("runtime blob size overflow")?;
                let bytes = objects.get_bounded(&usd_model::BlobId(reference.blob_id.clone()), size)?
                    .with_context(|| format!("missing runtime blob {}", reference.blob_id))?;
                ensure!(bytes.len() == size, "runtime blob byte size mismatch");
            }
        }
        Ok(())
    }

    /// Low-level descriptor persistence for unmanaged callers. Registered
    /// Projects must use the generation-bound publication path.
    pub(crate) fn publish(&self, descriptor: &ProjectCacheDescriptor) -> Result<PathBuf> {
        ensure!(!self.has_manifest(), "managed Project publication requires a desired generation");
        with_cache_lock(&self.project_root, "descriptor", || self.persist(descriptor, None, None))
    }

    pub(crate) fn reuse_generation(&self, identity: &ProjectCacheIdentity, generation: u64, revision: HashDigest) -> Result<bool> {
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(false); }
        with_cache_lock(&self.project_root, "descriptor", || {
            if !self.matches_desired(identity, generation, revision)? { return Ok(false); }
            let path = self.descriptor_path(identity)?;
            if !qualify_cache_file(&path, false)? { return Ok(false); }
            let bytes = fs::read(&path)?;
            let Ok(record) = serde_json::from_slice::<PublishedProjectCache>(&bytes) else { return Ok(false); };
            if record.revision_identity != Some(revision) || record.descriptor.identity != *identity
                || !matches!(record.descriptor.state, ProjectCacheState::Ready | ProjectCacheState::Empty) {
                return Ok(false);
            }
            if record.descriptor.validate().is_err() || self.verify_runtime(&record.descriptor).is_err() { return Ok(false); }
            self.publish_generation(&record.descriptor, generation, revision)
        })
    }

    pub(crate) fn publish_fallback(&self, descriptor: &ProjectCacheDescriptor, generation: u64) -> Result<bool> {
        ensure!(
            descriptor.state == ProjectCacheState::FallbackRequired && descriptor.runtime.is_none(),
            "fallback publication is restricted to FallbackRequired descriptors without runtime"
        );
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(false); }
        with_cache_lock(&self.project_root, "descriptor", || {
            if TombstoneStore::new(&self.project_root).is_retired_at(&descriptor.identity.target, generation)? {
                return Ok(false);
            }
            if !PendingIntentStore::new(&self.project_root)
                .load(&descriptor.identity.target)?
                .is_some_and(|desired| {
                    desired.generation == generation
                        && desired.config_hash == descriptor.identity.config_hash
                })
            {
                return Ok(false);
            }
            descriptor.validate()?;
            self.persist(descriptor, Some(generation), None)?;
            Ok(true)
        })
    }

    pub(crate) fn publish_generation(&self, descriptor: &ProjectCacheDescriptor, generation: u64, revision: HashDigest) -> Result<bool> {
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(false); }
        with_cache_lock(&self.project_root, "descriptor", || {
            if !self.matches_desired(&descriptor.identity, generation, revision)? { return Ok(false); }
            if !PendingIntentStore::new(&self.project_root).load(&descriptor.identity.target)?
                .is_some_and(|desired| desired.generation == generation) { return Ok(false); }
            descriptor.validate()?;
            self.verify_runtime(descriptor)?;
            self.persist(descriptor, Some(generation), Some(revision))?;
            if matches!(descriptor.state, ProjectCacheState::Ready | ProjectCacheState::Empty) {
                PendingIntentStore::new(&self.project_root).acknowledge_if_generation(
                    &descriptor.identity.target, generation, Some(revision))?;
            }
            Ok(true)
        })
    }

    fn persist(&self, descriptor: &ProjectCacheDescriptor, generation: Option<u64>, revision_identity: Option<HashDigest>) -> Result<PathBuf> {
        descriptor.validate()?;
        qualify_cache_directory(&self.descriptors, true)?;
        let path = self.descriptor_path(&descriptor.identity)?;
        let record = PublishedProjectCache { descriptor: descriptor.clone(), generation, revision_identity };
        let bytes = serde_json::to_vec_pretty(&record).context("encode Project cache descriptor")?;
        let temporary = self.descriptors.join(format!(".descriptor.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic(&temporary, &path, &bytes).context("publish Project cache descriptor")?;
        Ok(path)
    }

    pub(crate) fn load(&self, identity: &ProjectCacheIdentity) -> Result<Option<ProjectCacheDescriptor>> {
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(None); }
        with_cache_lock(&self.project_root, "descriptor", || {
            if super::sync::cache_invalidation_barrier(&self.project_root)? { return Ok(None); }
            let path = self.descriptor_path(identity)?;
            if !qualify_cache_file(&path, false)? { return Ok(None); }
            let record: PublishedProjectCache = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("decode Project cache descriptor {}", path.display()))?;
            let descriptor = record.descriptor;
            ensure!(descriptor.identity == *identity, "Project cache descriptor identity does not match its lookup");
            descriptor.validate()?;
            if self.has_manifest() || record.generation.is_some() {
                let Some(generation) = record.generation else { return Ok(None); };
                if let Some(revision) = record.revision_identity {
                    if !self.matches_desired(identity, generation, revision)? { return Ok(None); }
                    self.verify_runtime(&descriptor)?;
                } else if descriptor.state == ProjectCacheState::FallbackRequired {
                    if TombstoneStore::new(&self.project_root).is_retired_at(&identity.target, generation)? {
                        return Ok(None);
                    }
                    if !PendingIntentStore::new(&self.project_root)
                        .load_record(&identity.target)?
                        .is_some_and(|desired| {
                            desired.generation == generation
                                && desired.config_hash == identity.config_hash
                        })
                    {
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                }
            } else if TombstoneStore::new(&self.project_root).reactivation_floor(&identity.target)? > 0 {
                return Ok(None);
            }
            Ok(Some(descriptor))
        })
    }

    /// Logical retirement is durable before the legacy descriptor is removed.
    /// Legacy payload objects are deliberately not physically collected here.
    pub(crate) fn retire_target(&self, target: &ProjectCacheTarget) -> Result<()> {
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(()); }
        with_cache_lock(&self.project_root, "retirement", || {
            let pending = PendingIntentStore::new(&self.project_root);
            let tombstones = TombstoneStore::new(&self.project_root);
            let floor = pending.generation_floor(target)?.max(tombstones.reactivation_floor(target)?).saturating_add(1);
            tombstones.retire(target, floor)?;
            pending.remove(target)?;
            self.remove_target(target)?;
            Ok(())
        })
    }

    /// Remove every derived descriptor for one deleted Project target while
    /// leaving content-addressed payload objects available for reuse.
    pub(crate) fn remove_target(&self, target: &ProjectCacheTarget) -> Result<usize> {
        if super::sync::qualified_cache_root(&self.project_root, false)?.is_none() { return Ok(0); }
        with_cache_lock(&self.project_root, "descriptor", || self.remove_target_locked(target))
    }

    fn remove_target_locked(&self, target: &ProjectCacheTarget) -> Result<usize> {
        if !qualify_cache_directory(&self.descriptors, false)? { return Ok(0); }
        let entries = match fs::read_dir(&self.descriptors) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "read Project cache descriptors {}",
                        self.descriptors.display()
                    )
                });
            }
        };
        let mut removed = 0;
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if !qualify_cache_file(&path, false)? { continue; }
            let bytes = fs::read(&path)?;
            let descriptor: ProjectCacheDescriptor = match serde_json::from_slice(&bytes) {
                Ok(descriptor) => descriptor,
                Err(_) => continue,
            };
            if descriptor.identity.target == *target {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/project/cache/descriptor_tests.rs"]
mod tests;
