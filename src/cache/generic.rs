//! Generic, Project-free and Git-free USD cache identity and render-mesh cache.
//!
//! Any standalone USD entry file can derive a fail-closed [`SourceIdentity`] and
//! [`CacheIdentity`], persist real encoded Bevy [`Mesh`] payloads via
//! [`FilesystemBlobStore`] + `mesh_codec`/`mesh_v3`, and atomically publish and
//! hydrate validated descriptors into `usd_bevy::ProjectionSeed` under any
//! caller-supplied cache directory without requiring a managed Project
//! manifest, `ProjectStorageLayout`, or Git repository.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use bevy::mesh::Mesh;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use usd_model::{BlobId, HashDigest};
use uuid::Uuid;

use super::blobs::{
    BlobStore, FilesystemBlobStore, decode_mesh_payload, mesh_payload_version,
    prepare_mesh_payload, read_mesh_payload,
};

/// Format version 2 requires real encoded render-mesh payloads (`NMSH`/`NMS3`)
/// persisted in [`FilesystemBlobStore`]; v1 identity-only records are rejected
/// as misses.
pub(crate) const GENERIC_CACHE_FORMAT_VERSION: u32 = 2;
const GENERIC_DESCRIPTORS_DIR: &str = "generic-descriptors";
const GENERIC_OBJECTS_DIR: &str = "objects";

/// Neutral, Project-independent identity of a USD source entry and its
/// verified reachable dependency closure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SourceIdentity {
    pub(crate) entry_path: PathBuf,
    pub(crate) closure_digest: HashDigest,
    pub(crate) resolver_digest: HashDigest,
}

/// Generic cache key binding source closure identity, render/build recipe, and
/// cache format version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CacheIdentity {
    pub(crate) source: SourceIdentity,
    pub(crate) recipe_digest: HashDigest,
    pub(crate) format_version: u32,
}

impl CacheIdentity {
    pub(crate) fn new(source: SourceIdentity, recipe_digest: HashDigest) -> Self {
        Self {
            source,
            recipe_digest,
            format_version: GENERIC_CACHE_FORMAT_VERSION,
        }
    }

    pub(crate) fn storage_key(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"nemeton.generic-cache-identity.v2\0");
        let entry = self.source.entry_path.to_string_lossy();
        hasher.update(&(entry.len() as u64).to_le_bytes());
        hasher.update(entry.as_bytes());
        hasher.update(self.source.closure_digest.as_bytes());
        hasher.update(self.source.resolver_digest.as_bytes());
        hasher.update(self.recipe_digest.as_bytes());
        hasher.update(&self.format_version.to_le_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

/// Default recipe digest for standalone generic mesh caching.
pub(crate) fn default_standalone_recipe_digest() -> HashDigest {
    HashDigest::new(*blake3::hash(b"nemeton.standalone-mesh-cache.v2").as_bytes())
}

/// One validated mesh payload entry inside a generic standalone cache record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct GenericMeshEntry {
    pub(crate) prim_path: String,
    pub(crate) blob_id: BlobId,
    pub(crate) byte_len: u64,
    pub(crate) payload_version: u16,
}

/// Validated handle to a reusable generic render-mesh cache entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheHandle {
    pub(crate) entry_path: PathBuf,
    pub(crate) descriptor_path: PathBuf,
    pub(crate) blob_root: PathBuf,
    pub(crate) meshes: Vec<GenericMeshEntry>,
    pub(crate) hydrated_from_cache: bool,
    pub(crate) format_version: u32,
}

/// Optional Bevy resource installing a standalone generic cache provider for
/// non-Project stage openings.
#[derive(Resource, Clone, Debug)]
pub(crate) struct StandaloneCacheProviderResource {
    pub(crate) store: StandaloneCacheStore,
    pub(crate) recipe_digest: HashDigest,
}

/// Active validated standalone cache handle observed for the current generic
/// stage opening (absent on cache miss before publication, disabled cache, or
/// ineligible source).
#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveStandaloneCacheHandle(pub(crate) CacheHandle);

/// Pending post-projection cache publication bound to a specific standalone
/// stage identity, unedited authoring generation, and activation generation so
/// stale or edited openings never publish.
#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingStandaloneCachePublish {
    pub(crate) identity: CacheIdentity,
    pub(crate) stage_identity: (u64, u64),
    pub(crate) authoring_generation: usd_bevy::LiveRevision,
    pub(crate) generation: u64,
}

/// Generic cache lookup boundary usable by standalone and managed viewers alike.
pub(crate) trait CacheProvider {
    fn lookup(&self, identity: &CacheIdentity) -> Result<Option<CacheHandle>>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GenericCacheDescriptorRecord {
    format_version: u32,
    identity: CacheIdentity,
    meshes: Vec<GenericMeshEntry>,
}

/// Atomic byte write helper owned by `cache` (matching `manifest_store`'s
/// proven durability pattern): creates a fresh exclusive temp file, writes and
/// syncs all bytes, closes the handle, then promotes via `fs::rename` without
/// deleting `final_path` beforehand. On any error, cleans only the owned temp
/// file and preserves any pre-existing valid target intact.
pub(crate) fn write_bytes_atomic(
    temporary_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<()> {
    super::sync::write_bytes_atomic_durable(temporary_path, final_path, bytes)
}

/// Computes a fail-closed, Project-free and Git-free [`SourceIdentity`] for any
/// standalone USD entry file using OpenUSD `Stage::open` and
/// `all_dependencies()`. Any stage-open failure, dependency-discovery failure,
/// non-UTF8 path, unresolved required dependency, or unreadable file returns an
/// explicit error so an incomplete closure can never produce a reusable key.
pub(crate) fn compute_standalone_source_identity(entry_path: &Path) -> Result<SourceIdentity> {
    let live = usd_bevy::LiveStage::open_document(entry_path)
        .with_context(|| format!("capture complete USD source {}", entry_path.display()))?;
    source_identity_from_opened_stage(&live)
}

/// Hash exactly the sealed inputs of the stage that will consume the cache or
/// produce its payloads. Package-entry buffers are covered by their outer archive.
/// Never reopen a mutable dependency merely to label an already opened stage.
pub(crate) fn source_identity_from_opened_stage(live: &usd_bevy::LiveStage) -> Result<SourceIdentity> {
    let entry_path = live.opened_source_path().context("USD stage has no sealed file source")?;
    let files = live.opened_source_files().context("USD stage has no sealed dependency bytes")?;
    ensure!(files.contains_key(&entry_path), "sealed USD source is missing its entry");
    let composition = super::identity::compute_stage_composition_context(&live.stage)?;
    let mut closure_hasher = blake3::Hasher::new();
    closure_hasher.update(b"nemeton.standalone-sealed-closure.v3\0");
    closure_hasher.update(composition.as_bytes());
    for (path, bytes) in files {
        let key = path.to_str().context("sealed USD dependency path must be UTF-8")?;
        let bytes: &[u8] = bytes.as_ref();
        closure_hasher.update(&(key.len() as u64).to_le_bytes());
        closure_hasher.update(key.as_bytes());
        closure_hasher.update(&(bytes.len() as u64).to_le_bytes());
        closure_hasher.update(bytes);
    }
    let base_dir = entry_path.parent().context("sealed USD source has no parent")?;
    let mut resolver_hasher = blake3::Hasher::new();
    resolver_hasher.update(b"nemeton.sealed-resolver.v4\0");
    let bindings = live.opened_resolver_bindings()?.context("USD stage has no sealed resolver bindings")?;
    resolver_hasher.update(&(bindings.len() as u64).to_le_bytes());
    resolver_hasher.update(&bindings);
    resolver_hasher.update(base_dir.to_str().context("USD resolver root must be UTF-8")?.as_bytes());
    Ok(SourceIdentity {
        entry_path,
        closure_digest: HashDigest::new(*closure_hasher.finalize().as_bytes()),
        resolver_digest: HashDigest::new(*resolver_hasher.finalize().as_bytes()),
    })
}

/// Standalone, Project-free persistent render-mesh cache store rooted at any
/// caller-chosen directory. Backed by [`FilesystemBlobStore`] + `mesh_codec`
/// (`NMSH`/`NMS3`); corrupt descriptors, wrong payload formats, or
/// missing/corrupt blobs return `Ok(None)` so canonical USD remains
/// authoritative and caches rebuild safely.
#[derive(Clone, Debug)]
pub(crate) struct StandaloneCacheStore {
    root: PathBuf,
}

impl StandaloneCacheStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn blob_root(&self) -> PathBuf {
        self.root.join(GENERIC_OBJECTS_DIR)
    }

    pub(crate) fn descriptor_path(&self, identity: &CacheIdentity) -> PathBuf {
        self.root
            .join(GENERIC_DESCRIPTORS_DIR)
            .join(format!("{}.json", identity.storage_key()))
    }

    /// Encodes each projected Bevy [`Mesh`] with the real `NMSH`/`NMS3` codec,
    /// persists all blobs into [`FilesystemBlobStore`] first, and atomically
    /// publishes the descriptor only after every mesh blob succeeds.
    pub(crate) fn publish_meshes(
        &self,
        identity: &CacheIdentity,
        meshes: &[(String, &Mesh)],
    ) -> Result<CacheHandle> {
        ensure!(
            !meshes.is_empty(),
            "refusing to publish empty standalone render-mesh cache"
        );
        let blob_root = self.blob_root();
        let blob_store = FilesystemBlobStore::new(blob_root.clone())
            .context("initialize standalone cache blob store")?;

        let mut entries = Vec::with_capacity(meshes.len());
        for (prim_path, mesh) in meshes {
            let prepared = prepare_mesh_payload(mesh)
                .with_context(|| format!("encode standalone mesh payload for {prim_path}"))?;
            let payload_version = mesh_payload_version(&prepared.bytes)?;
            let byte_len = prepared.bytes.len() as u64;
            let blob_id = blob_store
                .put(&prepared.bytes)
                .with_context(|| format!("persist standalone mesh blob for {prim_path}"))?;
            ensure!(
                blob_id == prepared.blob_id,
                "standalone mesh blob digest mismatch for {prim_path}"
            );
            entries.push(GenericMeshEntry {
                prim_path: prim_path.clone(),
                blob_id,
                byte_len,
                payload_version,
            });
        }
        entries.sort_by(|left, right| left.prim_path.cmp(&right.prim_path));

        let descriptor_path = self.descriptor_path(identity);
        let temp_path =
            descriptor_path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        let record = GenericCacheDescriptorRecord {
            format_version: identity.format_version,
            identity: identity.clone(),
            meshes: entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&record)
            .context("serialize generic cache descriptor")?;
        write_bytes_atomic(&temp_path, &descriptor_path, &bytes)?;
        Ok(CacheHandle {
            entry_path: identity.source.entry_path.clone(),
            descriptor_path,
            blob_root,
            meshes: entries,
            hydrated_from_cache: false,
            format_version: identity.format_version,
        })
    }

    /// Validates the descriptor, verifies every referenced mesh blob's digest,
    /// byte length, and codec version, and decodes the real Bevy [`Mesh`]
    /// assets for hydration into `usd_bevy::ProjectionSeed`.
    pub(crate) fn hydrate_meshes(
        &self,
        identity: &CacheIdentity,
    ) -> Result<Option<(CacheHandle, Vec<(String, Mesh)>)>> {
        let descriptor_path = self.descriptor_path(identity);
        if !descriptor_path.is_file() {
            return Ok(None);
        }
        let Ok(bytes) = fs::read(&descriptor_path) else {
            return Ok(None);
        };
        let Ok(record) = serde_json::from_slice::<GenericCacheDescriptorRecord>(&bytes) else {
            return Ok(None);
        };
        if record.format_version != GENERIC_CACHE_FORMAT_VERSION
            || record.identity != *identity
            || record.meshes.is_empty()
        {
            return Ok(None);
        }
        let blob_root = self.blob_root();
        let Ok(blob_store) = FilesystemBlobStore::new(blob_root.clone()) else {
            return Ok(None);
        };
        let mut decoded_meshes = Vec::with_capacity(record.meshes.len());
        for entry in &record.meshes {
            let Ok(Some(blob_bytes)) = read_mesh_payload(&blob_store, &entry.blob_id) else {
                return Ok(None);
            };
            if blob_bytes.len() as u64 != entry.byte_len {
                return Ok(None);
            }
            let Ok(version) = mesh_payload_version(&blob_bytes) else {
                return Ok(None);
            };
            if version != entry.payload_version {
                return Ok(None);
            }
            let Ok(mesh) = decode_mesh_payload(&blob_bytes) else {
                return Ok(None);
            };
            decoded_meshes.push((entry.prim_path.clone(), mesh));
        }
        Ok(Some((
            CacheHandle {
                entry_path: identity.source.entry_path.clone(),
                descriptor_path,
                blob_root,
                meshes: record.meshes,
                hydrated_from_cache: true,
                format_version: record.format_version,
            },
            decoded_meshes,
        )))
    }

    /// Inspect the lifecycle state of a standalone cache entry without
    /// mutating canonical USD files.
    pub(crate) fn inspect_state(
        &self,
        identity: &CacheIdentity,
    ) -> super::sync::CacheLifecycleState {
        let descriptor_path = self.descriptor_path(identity);
        if !descriptor_path.is_file() {
            return super::sync::CacheLifecycleState::Missing;
        }
        match self.hydrate_meshes(identity) {
            Ok(Some(_)) => super::sync::CacheLifecycleState::Ready,
            Ok(None) | Err(_) => super::sync::CacheLifecycleState::Failed,
        }
    }

    /// Retire a standalone cache descriptor while leaving shared CAS blobs
    /// intact for other identities.
    pub(crate) fn retire(&self, identity: &CacheIdentity) -> Result<()> {
        let descriptor_path = self.descriptor_path(identity);
        match fs::remove_file(&descriptor_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "retire standalone cache descriptor {}",
                    descriptor_path.display()
                )
            }),
        }
    }
}

impl CacheProvider for StandaloneCacheStore {
    fn lookup(&self, identity: &CacheIdentity) -> Result<Option<CacheHandle>> {
        Ok(self.hydrate_meshes(identity)?.map(|(handle, _)| handle))
    }
}
