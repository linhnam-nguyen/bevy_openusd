//! Immutable revision retention, shared CAS blob reuse, and reader/builder
//! pin leases protecting active and historical cache artifacts from GC.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_model::{BlobId, HashDigest};
use uuid::Uuid;

use super::blobs::{BlobStore, FilesystemBlobStore};
use super::contract::{
    SceneCacheDescriptorV3, SceneCacheIndex, SceneSpatialIndex,
};
use super::identity::CacheRevisionIdentity;
use super::sync::{is_path_alias, qualify_cache_entry, qualified_cache_root, unix_now_secs, with_cache_lock, write_bytes_atomic_durable};
use crate::document::runtime::payload::RuntimeMaterialBlob;
use crate::project::storage::ProjectStorageLayout;

const RETAINED_REVISION_SCHEMA_VERSION: u16 = 1;
const LEGACY_PIN_RECORD_SCHEMA_VERSION: u16 = 1;
const PIN_RECORD_SCHEMA_VERSION: u16 = 2;

fn valid_cache_digest(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Immutable, content-addressed Scene revision record retained independently
/// of the mutable current pointer so historical revisions and branches can
/// reuse verified descriptors and shared CAS blobs without rebuilding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RetainedRevisionRecord {
    pub(crate) schema_version: u16,
    pub(crate) revision_key: String,
    pub(crate) identity: CacheRevisionIdentity,
    pub(crate) descriptor: SceneCacheDescriptorV3,
    pub(crate) index_digest: HashDigest,
    pub(crate) spatial_digest: HashDigest,
    pub(crate) referenced_blobs: Vec<BlobId>,
    pub(crate) created_at_unix_secs: u64,
}

impl RetainedRevisionRecord {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == RETAINED_REVISION_SCHEMA_VERSION,
            "unsupported retained revision schema version {}",
            self.schema_version
        );
        ensure!(
            self.revision_key == self.identity.storage_key(),
            "retained revision key does not match identity digest"
        );
        self.descriptor.validate()?;
        ensure!(self.identity.target == (super::contract::ProjectCacheTarget::Scene {
            id: self.descriptor.scene_id.to_string(),
        }), "retained revision target mismatch");
        ensure!(self.descriptor.revision_identity == Some(self.identity.digest()),
            "retained revision descriptor identity mismatch");
        ensure!(self.descriptor.source_content_hash == Some(self.identity.source_closure_digest),
            "retained revision source identity mismatch");
        ensure!(
            self.descriptor.index_digest == self.index_digest,
            "retained revision index digest mismatch"
        );
        ensure!(
            self.descriptor.spatial_digest == Some(self.spatial_digest),
            "retained revision spatial digest mismatch"
        );
        Ok(())
    }
}

/// Verified immutable revision loaded from disk.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedRetainedRevision {
    pub(crate) record: RetainedRevisionRecord,
    pub(crate) index: SceneCacheIndex,
    pub(crate) spatial: SceneSpatialIndex,
}

/// Store for immutable Scene cache revisions and shared CAS payload objects.
#[derive(Clone, Debug)]
pub(crate) struct ImmutableRevisionStore {
    layout: ProjectStorageLayout,
}

impl ImmutableRevisionStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            layout: ProjectStorageLayout::new(project_root),
        }
    }

    pub(crate) fn shared_object_store(&self) -> Result<FilesystemBlobStore> {
        FilesystemBlobStore::new(self.layout.shared_cache_objects_dir())
    }

    pub(crate) fn revision_dir(&self, revision_key: &str) -> PathBuf {
        self.layout.cache_revisions_dir().join(revision_key)
    }

    /// Collect all CAS `BlobId`s referenced by a `SceneCacheIndex`, including
    /// geometry, material, animation, and transitive texture blobs referenced
    /// inside material payloads.
    pub(crate) fn collect_referenced_blobs(
        blob_store: &FilesystemBlobStore,
        index: &SceneCacheIndex,
    ) -> Result<Vec<BlobId>> {
        Self::try_collect_referenced_blobs(blob_store, index)?
            .context("missing or malformed material reference closure")
    }

    fn try_collect_referenced_blobs(
        blob_store: &FilesystemBlobStore,
        index: &SceneCacheIndex,
    ) -> Result<Option<Vec<BlobId>>> {
        let mut blobs = BTreeSet::new();
        for entry in &index.entries {
            if let Some(geometry) = &entry.geometry { blobs.insert(geometry.blob_id.clone()); }
            if let Some(animation) = &entry.animation { blobs.insert(animation.blob_id.clone()); }
            if let Some(material) = &entry.material {
                blobs.insert(material.blob_id.clone());
                let Some(bytes) = Self::read_repairable_blob(blob_store, &material.blob_id)? else {
                    return Ok(None);
                };
                let Ok(parsed) = serde_json::from_slice::<RuntimeMaterialBlob>(&bytes) else {
                    return Ok(None);
                };
                if parsed.validate().is_err() { return Ok(None); }
                for texture_id in [
                    parsed.textures.base_color.as_deref(), parsed.textures.normal.as_deref(),
                    parsed.textures.metallic_roughness.as_deref(), parsed.textures.emissive.as_deref(),
                    parsed.textures.occlusion.as_deref(),
                ].into_iter().flatten() {
                    blobs.insert(BlobId(texture_id.to_owned()));
                }
            }
        }
        Ok(Some(blobs.into_iter().collect()))
    }

    // A qualified missing or digest-corrupt blob is repairable. Access denial,
    // unexpected filesystem types and alias failures remain operational errors.
    fn read_repairable_blob(store: &FilesystemBlobStore, id: &BlobId) -> Result<Option<Vec<u8>>> {
        if !valid_cache_digest(&id.0) {
            return Ok(None);
        }
        match store.get(id) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.downcast_ref::<super::blobs::BlobDigestMismatch>().is_some() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Verify that every blob in `blob_ids` exists in `blob_store` and passes
    /// BLAKE3 digest verification.
    pub(crate) fn verify_blobs_present(
        blob_store: &FilesystemBlobStore,
        blob_ids: &[BlobId],
    ) -> Result<()> {
        for blob_id in blob_ids {
            ensure!(
                blob_store.contains(blob_id)?,
                "required cache blob {} is missing or corrupt",
                blob_id.0
            );
        }
        Ok(())
    }

    /// Copy/deduplicate every referenced blob from `source_store` into the
    /// shared CAS store and verify all digests before persisting an immutable
    /// revision record.
    pub(crate) fn store_verified_revision(
        &self,
        identity: &CacheRevisionIdentity,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
        source_store: &FilesystemBlobStore,
    ) -> Result<RetainedRevisionRecord> {
        with_cache_lock(self.layout.root(), "revision", || {
            self.store_verified_revision_locked(identity, descriptor, index, spatial, source_store, None)
        })
    }

    pub(crate) fn store_verified_revision_for_build(
        &self,
        identity: &CacheRevisionIdentity,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
        source_store: &FilesystemBlobStore,
        build_pin: &CachePinGuard,
    ) -> Result<RetainedRevisionRecord> {
        with_cache_lock(self.layout.root(), "revision", || {
            self.store_verified_revision_locked(identity, descriptor, index, spatial, source_store, Some(build_pin.pin_id()))
        })
    }

    fn store_verified_revision_locked(
        &self,
        identity: &CacheRevisionIdentity,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
        source_store: &FilesystemBlobStore,
        own_pin: Option<&str>,
    ) -> Result<RetainedRevisionRecord> {
        descriptor.validate()?;
        let revision_key = identity.storage_key();
        let revision_dir = self.revision_dir(&revision_key);
        let damaged = match fs::symlink_metadata(&revision_dir) {
            Ok(_) => {
                if let Some(verified) = self.load_verified_revision_by_key(&revision_key, None)? {
                    ensure!(verified.record.identity == *identity, "different valid identity under immutable revision key");
                    return Ok(verified.record);
                }
                let root = qualified_cache_root(self.layout.root(), false)?.context("missing cache root")?;
                // A missing/hash-invalid/structurally invalid record is repairable
                // only after qualifying its entire existing subtree.
                Self::qualify_revision_subtree(&root, &revision_dir)?;
                self.ensure_repair_unleased(own_pin)?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error).context("inspect immutable revision before retention"),
        };
        index.validate(descriptor.scene_id, descriptor.generation)?;
        spatial.validate(descriptor.scene_id, descriptor.generation)?;

        let shared_store = self.shared_object_store()?;
        let referenced_blobs = Self::collect_referenced_blobs(source_store, index)?;
        for blob_id in &referenced_blobs {
            let needs_copy = match shared_store.get(blob_id) {
                Ok(Some(_)) => false,
                Ok(None) => true,
                Err(error) if error.downcast_ref::<super::blobs::BlobDigestMismatch>().is_some() => {
                    self.ensure_repair_unleased(own_pin)?;
                    true
                }
                Err(error) => return Err(error),
            };
            if needs_copy {
                let bytes = source_store.get(blob_id)?.with_context(|| {
                    format!("missing source blob {} during revision retention", blob_id.0)
                })?;
                let stored = shared_store.put(&bytes)?;
                ensure!(
                    stored == *blob_id,
                    "shared CAS digest mismatch for {}",
                    blob_id.0
                );
            }
        }
        Self::verify_blobs_present(&shared_store, &referenced_blobs)?;

        let index_bytes = serde_json::to_vec(index).context("encode retained Scene index")?;
        let spatial_bytes =
            serde_json::to_vec(spatial).context("encode retained Scene spatial index")?;
        let index_digest = HashDigest::new(*blake3::hash(&index_bytes).as_bytes());
        let spatial_digest = HashDigest::new(*blake3::hash(&spatial_bytes).as_bytes());

        let mut retained_descriptor = descriptor.clone();
        retained_descriptor.index_digest = index_digest;
        retained_descriptor.spatial_digest = Some(spatial_digest);
        retained_descriptor.revision_identity = Some(identity.digest());

        let revisions_root = self.layout.cache_revisions_dir();
        super::sync::qualify_cache_directory(&revisions_root, true)?;
        let cache_root = qualified_cache_root(self.layout.root(), false)?.context("missing cache root")?;
        qualify_cache_entry(&cache_root, &revisions_root)?;
        let staging = revisions_root.join(format!(".revision.{}.tmp", Uuid::new_v4()));
        fs::create_dir(&staging).context("create private immutable revision staging directory")?;
        let index_path = staging.join("index.bin");
        let index_tmp = staging.join(format!(".index.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic_durable(&index_tmp, &index_path, &index_bytes)?;

        let spatial_path = staging.join("spatial.bin");
        let spatial_tmp = staging.join(format!(".spatial.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic_durable(&spatial_tmp, &spatial_path, &spatial_bytes)?;

        let record = RetainedRevisionRecord {
            schema_version: RETAINED_REVISION_SCHEMA_VERSION,
            revision_key,
            identity: identity.clone(),
            descriptor: retained_descriptor,
            index_digest,
            spatial_digest,
            referenced_blobs,
            created_at_unix_secs: unix_now_secs(),
        };
        record.validate()?;

        let record_path = staging.join("record.json");
        let record_tmp = staging.join(format!(".record.{}.tmp", Uuid::new_v4()));
        let record_bytes =
            serde_json::to_vec_pretty(&record).context("encode retained revision record")?;
        write_bytes_atomic_durable(&record_tmp, &record_path, &record_bytes)?;
        if damaged {
            // All replacement bytes and their complete reference set are now
            // staged. Preserve the damaged directory for diagnosis; never
            // overwrite a valid immutable record or sweep quarantined data.
            self.ensure_repair_unleased(own_pin)?;
            Self::qualify_revision_subtree(&cache_root, &revision_dir)?;
            let quarantine = revisions_root.join(format!(".quarantine.{}", Uuid::new_v4()));
            ensure!(!quarantine.exists(), "revision quarantine already exists");
            fs::rename(&revision_dir, &quarantine).context("quarantine damaged immutable revision")?;
            log::warn!("quarantined damaged cache revision {} at {}", record.revision_key, quarantine.display());
        }
        ensure!(!revision_dir.exists(), "immutable revision appeared before commit");
        fs::rename(&staging, &revision_dir).context("commit complete immutable revision")?;
        if let Ok(directory) = File::open(&revisions_root) { let _ = directory.sync_all(); }
        Ok(record)
    }

    fn ensure_repair_unleased(&self, own_pin: Option<&str>) -> Result<()> {
        let active = load_active_pins(self.layout.root())?;
        ensure!(active.iter().all(|pin| Some(pin.pin_id.as_str()) == own_pin),
            "damaged immutable revision repair deferred while other cache leases are active");
        Ok(())
    }

    fn qualify_revision_subtree(root: &Path, directory: &Path) -> Result<()> {
        ensure!(qualify_cache_entry(root, directory)?.is_dir(), "invalid revision directory");
        let mut directories = vec![directory.to_path_buf()];
        while let Some(dir) = directories.pop() {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                let metadata = qualify_cache_entry(root, &path)?;
                if metadata.is_dir() { directories.push(path); }
                else { ensure!(metadata.is_file(), "unexpected retained revision entry type"); }
            }
        }
        Ok(())
    }

    /// Look up and fully verify an immutable retained revision matching
    /// `identity`. Returns `Ok(None)` if absent or incomplete/corrupt.
    pub(crate) fn load_verified_revision(
        &self,
        identity: &CacheRevisionIdentity,
    ) -> Result<Option<VerifiedRetainedRevision>> {
        self.load_verified_revision_by_key(&identity.storage_key(), Some(identity))
    }

    pub(crate) fn load_verified_revision_by_key(
        &self,
        revision_key: &str,
        expected_identity: Option<&CacheRevisionIdentity>,
    ) -> Result<Option<VerifiedRetainedRevision>> {
        ensure!(valid_cache_digest(revision_key),
            "invalid immutable revision key");
        let revision_dir = self.revision_dir(revision_key);
        let Some(cache_root) = qualified_cache_root(self.layout.root(), false)? else { return Ok(None); };
        match fs::symlink_metadata(&revision_dir) {
            Ok(_) => ensure!(qualify_cache_entry(&cache_root, &revision_dir)?.is_dir(), "invalid retained revision directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect retained revision directory"),
        }
        let record_path = revision_dir.join("record.json");
        if !super::sync::qualify_cache_file(&record_path, false)? { return Ok(None); }
        let record_bytes = match fs::read(&record_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read retained revision {}", record_path.display()));
            }
        };
        let record: RetainedRevisionRecord = match serde_json::from_slice(&record_bytes) {
            Ok(record) => record,
            Err(_) => return Ok(None),
        };
        if record.validate().is_err() || record.revision_key != revision_key {
            return Ok(None);
        }
        if let Some(expected) = expected_identity
            && &record.identity != expected
        {
            return Ok(None);
        }

        let index_path = revision_dir.join("index.bin");
        if !super::sync::qualify_cache_file(&index_path, false)? { return Ok(None); }
        let index_bytes = match fs::read(index_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read retained revision index"),
        };
        if HashDigest::new(*blake3::hash(&index_bytes).as_bytes()) != record.index_digest {
            return Ok(None);
        }
        let index: SceneCacheIndex = match serde_json::from_slice(&index_bytes) {
            Ok(index) => index,
            Err(_) => return Ok(None),
        };
        if index
            .validate(record.descriptor.scene_id, record.descriptor.generation)
            .is_err()
        {
            return Ok(None);
        }

        let spatial_path = revision_dir.join("spatial.bin");
        if !super::sync::qualify_cache_file(&spatial_path, false)? { return Ok(None); }
        let spatial_bytes = match fs::read(spatial_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read retained revision spatial index"),
        };
        if HashDigest::new(*blake3::hash(&spatial_bytes).as_bytes()) != record.spatial_digest {
            return Ok(None);
        }
        let spatial: SceneSpatialIndex = match serde_json::from_slice(&spatial_bytes) {
            Ok(spatial) => spatial,
            Err(_) => return Ok(None),
        };
        if spatial
            .validate(record.descriptor.scene_id, record.descriptor.generation)
            .is_err()
        {
            return Ok(None);
        }

        let shared_store = self.shared_object_store()?;
        let Some(derived) = Self::try_collect_referenced_blobs(&shared_store, &index)? else {
            return Ok(None);
        };
        if derived != record.referenced_blobs { return Ok(None); }
        for id in &derived {
            if Self::read_repairable_blob(&shared_store, id)?.is_none() { return Ok(None); }
        }

        Ok(Some(VerifiedRetainedRevision {
            record,
            index,
            spatial,
        }))
    }

    /// Materialize all blobs referenced by a verified retained revision into
    /// `destination_store` (skipping any blob already present and valid).
    pub(crate) fn materialize_blobs_to(
        &self,
        referenced_blobs: &[BlobId],
        destination_store: &FilesystemBlobStore,
    ) -> Result<()> {
        let shared_store = self.shared_object_store()?;
        for blob_id in referenced_blobs {
            if Self::read_repairable_blob(destination_store, blob_id)?.is_some() {
                continue;
            }
            let bytes = shared_store.get(blob_id)?.with_context(|| {
                format!(
                    "shared CAS blob {} missing during revision materialization",
                    blob_id.0
                )
            })?;
            let stored = destination_store.put(&bytes)?;
            ensure!(
                stored == *blob_id,
                "materialized CAS blob digest mismatch for {}",
                blob_id.0
            );
        }
        Ok(())
    }

    /// Enumerate all retained revision records on disk. Returns an error if
    /// any revision directory is corrupt so GC fails closed rather than
    /// guessing reachability.
    pub(crate) fn list_revisions(&self) -> Result<Vec<RetainedRevisionRecord>> {
        let revisions_root = self.layout.cache_revisions_dir();
        if !super::sync::qualify_cache_directory(&revisions_root, false)? { return Ok(Vec::new()); }
        let entries = match fs::read_dir(&revisions_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "read immutable revisions directory {}",
                        revisions_root.display()
                    )
                });
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !is_path_alias(&metadata),
                "refusing symlink in immutable revisions directory: {}",
                path.display()
            );
            let name = path.file_name().and_then(|name| name.to_str())
                .context("non-UTF8 immutable revision entry")?;
            if !metadata.is_dir() {
                ensure!(!valid_cache_digest(name), "committed revision key is not a directory: {name}");
                continue;
            }
            // Interrupted private staging was never committed and is not a root.
            if (name.starts_with(".revision.") && name.ends_with(".tmp"))
                || name.strip_prefix(".quarantine.").is_some_and(|id| Uuid::parse_str(id).is_ok()) {
                continue;
            }
            let verified = self.load_verified_revision_by_key(name, None)?
                .with_context(|| format!("unverified retained revision {name}"))?;
            records.push(verified.record);
        }
        records.sort_by(|left, right| {
            right
                .descriptor
                .generation
                .cmp(&left.descriptor.generation)
                .then_with(|| right.created_at_unix_secs.cmp(&left.created_at_unix_secs))
                .then_with(|| left.revision_key.cmp(&right.revision_key))
        });
        Ok(records)
    }
}

/// Persisted pin record protecting active readers and in-flight builders
/// across processes from concurrent garbage collection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CachePinRecord {
    pub(crate) schema_version: u16,
    pub(crate) pin_id: String,
    pub(crate) owner_pid: u32,
    pub(crate) revision_keys: Vec<String>,
    pub(crate) blob_ids: Vec<BlobId>,
    pub(crate) created_at_unix_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) integrity: Option<HashDigest>,
}

impl CachePinRecord {
    fn validate_shape(&self) -> Result<()> {
        ensure!(matches!(self.schema_version, LEGACY_PIN_RECORD_SCHEMA_VERSION | PIN_RECORD_SCHEMA_VERSION),
            "unsupported cache pin schema version {}", self.schema_version);
        ensure!(Uuid::parse_str(&self.pin_id).is_ok(), "invalid cache pin id");
        for key in &self.revision_keys {
            ensure!(valid_cache_digest(key), "invalid cache pin revision key");
        }
        for id in &self.blob_ids {
            ensure!(valid_cache_digest(&id.0), "invalid cache pin blob id");
        }
        Ok(())
    }

    fn compute_integrity(&self) -> Result<HashDigest> {
        // The serialized tuple has explicit field boundaries and stable order;
        // it excludes only the integrity field itself. This is a corruption
        // checksum, not authentication against deliberate rewrites.
        let bytes = serde_json::to_vec(&(
            self.schema_version, &self.pin_id, self.owner_pid,
            &self.revision_keys, &self.blob_ids, self.created_at_unix_secs,
        )).context("encode cache pin integrity fields")?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"nemeton.cache-pin-integrity.v2\0");
        hasher.update(&bytes);
        Ok(HashDigest::new(*hasher.finalize().as_bytes()))
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.validate_shape()?;
        ensure!(self.schema_version == PIN_RECORD_SCHEMA_VERSION, "legacy cache pin has no root integrity proof");
        ensure!(self.integrity == Some(self.compute_integrity()?), "cache pin integrity mismatch");
        Ok(())
    }
}

/// A live lease holds an OS lock on a unique sidecar, while its immutable JSON
/// reference record remains readable on every platform. Closing the handle
/// releases the lease, including on process exit; Drop does not acquire a mutex
/// or remove files. A collector can distinguish abandoned leases from active
/// ones without PID reuse or a timeout heuristic.
#[derive(Debug)]
pub(crate) struct CachePinGuard {
    pin_id: String,
    _lease: File,
}

impl CachePinGuard {
    pub(crate) fn acquire(
        project_root: &Path,
        revision_keys: Vec<String>,
        blob_ids: Vec<BlobId>,
    ) -> Result<Self> {
        with_cache_lock(project_root, "pin", || {
            let pin_id = Uuid::new_v4().to_string();
            let pins_dir = ProjectStorageLayout::new(project_root).cache_pins_dir();
            super::sync::qualify_cache_directory(&pins_dir, true)?;
            let cache_root = qualified_cache_root(project_root, false)?.context("missing cache root")?;
            qualify_cache_entry(&cache_root, &pins_dir)?;
            let mut record = CachePinRecord {
                schema_version: PIN_RECORD_SCHEMA_VERSION,
                pin_id: pin_id.clone(),
                owner_pid: std::process::id(),
                revision_keys,
                blob_ids,
                created_at_unix_secs: unix_now_secs(),
                integrity: None,
            };
            record.integrity = Some(record.compute_integrity()?);
            record.validate()?;
            let lease_path = pins_dir.join(format!("{pin_id}.lease"));
            let lease = OpenOptions::new().read(true).write(true).create_new(true)
                .open(&lease_path).context("create unique cache pin lease")?;
            lease.lock().context("lock cache pin lease")?;
            let pin_file = pins_dir.join(format!("{pin_id}.json"));
            let temporary = pins_dir.join(format!(".pin.{pin_id}.tmp"));
            let bytes = serde_json::to_vec_pretty(&record).context("serialize cache pin record")?;
            write_bytes_atomic_durable(&temporary, &pin_file, &bytes)?;
            Ok(Self { pin_id, _lease: lease })
        })
    }

    pub(crate) fn pin_id(&self) -> &str { &self.pin_id }
}

/// Inspect leases under the same Project transaction as GC and pin admission.
/// Invalid or pre-migration records remain protected by aborting collection.
/// Stale valid lease files are not reachability roots; they can be cleaned by
/// an explicit maintenance operation after complete qualification.
pub(crate) fn load_active_pins(project_root: &Path) -> Result<Vec<CachePinRecord>> {
    let Some(cache_root) = qualified_cache_root(project_root, false)? else { return Ok(Vec::new()); };
    with_cache_lock(project_root, "pin", || {
        let pins_dir = ProjectStorageLayout::new(project_root).cache_pins_dir();
        if !super::sync::qualify_cache_directory(&pins_dir, false)? { return Ok(Vec::new()); }
        let entries = match fs::read_dir(&pins_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("read cache pin directory"),
        };
        qualify_cache_entry(&cache_root, &pins_dir)?;
        let mut active = Vec::new();
        let mut records = BTreeSet::new();
        let mut leases = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let extension = path.extension().and_then(|ext| ext.to_str());
            if !matches!(extension, Some("json" | "lease")) { continue; }
            ensure!(qualify_cache_entry(&cache_root, &path)?.is_file(), "invalid cache pin entry type");
            let name = path.file_stem().and_then(|name| name.to_str()).context("invalid cache pin locator")?;
            ensure!(Uuid::parse_str(name).is_ok(), "invalid cache pin locator");
            if extension == Some("lease") {
                leases.push((name.to_owned(), path));
                continue;
            }
            let record: CachePinRecord = serde_json::from_slice(&fs::read(&path)?)
                .context("decode cache pin record")?;
            record.validate_shape()?;
            ensure!(name == record.pin_id, "cache pin locator mismatch");
            let lease_path = pins_dir.join(format!("{}.lease", record.pin_id));
            ensure!(qualify_cache_entry(&cache_root, &lease_path)?.is_file(), "invalid cache pin lease");
            let lease = OpenOptions::new().read(true).write(true).open(&lease_path)?;
            records.insert(record.pin_id.clone());
            match lease.try_lock() {
                Ok(()) => {
                    // A legacy record can be ignored only when its exact lease
                    // is proved inactive. Do not rewrite or delete its bytes.
                    if record.schema_version == PIN_RECORD_SCHEMA_VERSION {
                        record.validate()?;
                    }
                }
                Err(TryLockError::WouldBlock) => {
                    record.validate().context("uncertain active cache pin root")?;
                    active.push(record);
                }
                Err(TryLockError::Error(error)) => return Err(error).context("qualify cache pin liveness"),
            }
        }
        // An interrupted admission can leave an unlocked sidecar without JSON.
        // A locked one without its record is unknown reachability, not absence.
        for (id, path) in leases {
            if records.contains(&id) { continue; }
            let lease = OpenOptions::new().read(true).write(true).open(&path)?;
            match lease.try_lock() {
                Ok(()) => {},
                Err(TryLockError::WouldBlock) => anyhow::bail!("active cache pin lease has no reference record: {id}"),
                Err(TryLockError::Error(error)) => return Err(error).context("qualify orphan cache pin lease"),
            }
        }
        Ok(active)
    })
}
