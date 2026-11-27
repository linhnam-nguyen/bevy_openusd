//! Cross-process synchronization, durable atomic publication, pending
//! preparation intents, tombstones, and canonical cache lifecycle states.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_model::HashDigest;
use uuid::Uuid;

use super::contract::ProjectCacheTarget;

use crate::project::storage::{CACHE_DIRECTORY, PROJECT_METADATA_DIRECTORY};
use crate::project::storage::ProjectStorageLayout;

const PENDING_INTENT_SCHEMA_VERSION: u16 = 1;

fn pending_by_default() -> bool { true }
const TOMBSTONE_SCHEMA_VERSION: u16 = 1;

/// Canonical six-state cache lifecycle model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CacheLifecycleState {
    Missing,
    Building,
    Ready,
    Stale,
    Failed,
    Retired,
}

impl fmt::Display for CacheLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Missing => "missing",
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Retired => "retired",
        };
        f.write_str(label)
    }
}

/// Durable atomic write: creates an exclusive temporary file, writes and syncs
/// all bytes, explicitly closes the handle before rename (required for reliable
/// replacement on Windows), and syncs the parent directory best-effort.
///
/// On any failure, removes only the owned temporary file and leaves any
/// pre-existing `final_path` untouched.
pub(crate) fn write_bytes_atomic_durable(
    temporary_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<()> {
    qualify_cache_file(final_path, true)?;
    ensure!(!qualify_cache_file(temporary_path, true)?, "cache temporary file already exists");
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)
            .with_context(|| format!("create temporary cache file {}", temporary_path.display()))?;
        created = true;
        file.write_all(bytes)
            .with_context(|| format!("write temporary cache file {}", temporary_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync temporary cache file {}", temporary_path.display()))?;
        drop(file);
        fs::rename(temporary_path, final_path).with_context(|| {
            format!(
                "publish temporary cache file {} as {}",
                temporary_path.display(),
                final_path.display()
            )
        })?;
        sync_parent_best_effort(final_path.parent());
        Ok(())
    })();

    if result.is_err() && created {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn sync_parent_best_effort(parent: Option<&Path>) {
    let Some(parent) = parent else {
        return;
    };
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

/// Return true for a link or a Windows reparse point, including junctions.
pub(crate) fn is_path_alias(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Establish the ordinary derived directory below the resolved Project root.
/// Never bless a directory merely because canonicalizing an alias succeeds.
pub(crate) fn qualified_cache_root(project_root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let project = fs::canonicalize(project_root).context("resolve Project root for cache")?;
    ensure!(project.is_dir(), "Project root is not a directory");
    let layout = ProjectStorageLayout::new(&project);
    let cache = layout.cache_dir();
    let suffix = cache.strip_prefix(&project).context("cache root outside Project")?;
    let mut current = project;
    for component in suffix.components() {
        let Component::Normal(name) = component else {
            anyhow::bail!("invalid Project cache directory component");
        };
        current.push(name);
        if create {
            match fs::create_dir(&current) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(error).context("create qualified cache directory"),
            }
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect cache directory component"),
        };
        ensure!(metadata.is_dir() && !is_path_alias(&metadata),
            "refusing aliased or non-directory cache component {}", current.display());
        ensure!(fs::canonicalize(&current)? == current,
            "cache directory component changed identity: {}", current.display());
    }
    Ok(Some(current))
}

/// Find the lexical Project root of a managed cache path. Standalone stores
/// have no Project layout and retain their independent directory contract.
fn managed_cache_layout(path: &Path) -> Option<(&Path, &Path)> {
    path.ancestors().find_map(|ancestor| {
        if ancestor.file_name()? != CACHE_DIRECTORY { return None; }
        let metadata = ancestor.parent()?;
        if metadata.file_name()? != PROJECT_METADATA_DIRECTORY { return None; }
        let project = metadata.parent()?;
        Some((if project.as_os_str().is_empty() { Path::new(".") } else { project }, ancestor))
    })
}

/// Walk a managed directory one component at a time. Never create a child
/// through an existing alias, even when the child itself does not exist.
/// The caller must obey the cooperating-writer contract in the cache docs.
pub(crate) fn qualify_cache_directory(path: &Path, create: bool) -> Result<bool> {
    let Some((project_root, lexical_root)) = managed_cache_layout(path) else {
        if create { fs::create_dir_all(path)?; }
        return match fs::metadata(path) {
            Ok(metadata) => { ensure!(metadata.is_dir(), "cache path is not a directory"); Ok(true) }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("inspect standalone cache directory"),
        };
    };
    let relative = path.strip_prefix(lexical_root)
        .context("managed cache directory is outside its lexical root")?;
    let Some(root) = qualified_cache_root(project_root, create)? else { return Ok(false); };
    let mut parent = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            anyhow::bail!("invalid managed cache directory component");
        };
        let child = parent.join(name);
        match fs::symlink_metadata(&child) {
            Ok(metadata) => ensure!(metadata.is_dir() && !is_path_alias(&metadata),
                "refusing aliased or non-directory cache ancestor {}", child.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                match fs::create_dir(&child) {
                    Ok(()) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                    Err(error) => return Err(error).context("create qualified cache child"),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("inspect managed cache directory"),
        }
        ensure!(qualify_cache_entry(&root, &child)?.is_dir(), "invalid managed cache directory");
        parent = child;
    }
    Ok(true)
}

/// Validate the full parent chain and existing leaf before opening a file.
/// A missing leaf is ordinary absence. Creation applies to the parent only.
pub(crate) fn qualify_cache_file(path: &Path, create_parent: bool) -> Result<bool> {
    let Some((project_root, _)) = managed_cache_layout(path) else {
        if create_parent {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
        }
        return match fs::metadata(path) {
            Ok(metadata) => { ensure!(metadata.is_file(), "cache path is not a regular file"); Ok(true) }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("inspect standalone cache file"),
        };
    };
    let parent = path.parent().context("cache file has no parent")?;
    ensure!(path.file_name().is_some(), "cache file has no name");
    if !qualify_cache_directory(parent, create_parent)? { return Ok(false); }
    let root = qualified_cache_root(project_root, false)?.context("missing cache root")?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            ensure!(qualify_cache_entry(&root, path)?.is_file(), "invalid cache file type");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("inspect managed cache file"),
    }
}

/// Validate every existing component from a candidate back to a previously
/// qualified cache root. All destructive callers also hold the transaction.
pub(crate) fn qualify_cache_entry(root: &Path, path: &Path) -> Result<fs::Metadata> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolve cache entry {}", path.display()))?;
    ensure!(canonical != root && canonical.starts_with(root),
        "cache entry outside qualified derived root: {}", path.display());
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!is_path_alias(&metadata), "refusing aliased cache entry {}", path.display());
    for ancestor in path.ancestors().skip(1) {
        let resolved = fs::canonicalize(ancestor)?;
        if resolved == root {
            let root_metadata = fs::symlink_metadata(ancestor)?;
            ensure!(root_metadata.is_dir() && !is_path_alias(&root_metadata), "cache root was replaced");
            return Ok(metadata);
        }
        ensure!(resolved.starts_with(root), "cache entry escaped derived root");
        let ancestor_metadata = fs::symlink_metadata(ancestor)?;
        ensure!(ancestor_metadata.is_dir() && !is_path_alias(&ancestor_metadata),
            "refusing alias in cache entry ancestry: {}", ancestor.display());
    }
    anyhow::bail!("cache entry has no qualified root ancestor")
}

static PROCESS_FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static HELD_CACHE_TRANSACTIONS: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

fn process_lock_for(path: PathBuf) -> Result<Arc<Mutex<()>>> {
    let mut locks = PROCESS_FILE_LOCKS.lock()
        .map_err(|_| anyhow::anyhow!("cache transaction registry poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path, Arc::downgrade(&lock));
    Ok(lock)
}

/// One Project-wide transaction protects pointer publication, intent changes,
/// pin admission, retention and GC. Existing scope names are diagnostic only;
/// they must never establish independent exclusion domains. The lock lives in
/// the qualified, unswept cache lock directory.
/// Nested calls by the owning thread join its transaction. Do not call external
/// callbacks or wait for another thread while inside a transaction.
pub(crate) fn with_cache_lock<T>(
    project_root: &Path,
    _scope_name: &str,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let root = qualified_cache_root(project_root, true)?.context("missing cache root")?;
    let lock_dir = root.join(".locks");
    qualify_cache_directory(&lock_dir, true)?;
    ensure!(qualify_cache_entry(&root, &lock_dir)?.is_dir(), "invalid cache lock directory");
    let lock_path = lock_dir.join("project.lock");
    if HELD_CACHE_TRANSACTIONS.with(|held| held.borrow().contains(&lock_path)) {
        return operation();
    }
    let process_lock = process_lock_for(lock_path.clone())?;
    let _process_guard = process_lock.lock()
        .map_err(|_| anyhow::anyhow!("cache transaction lock poisoned"))?;
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata) => ensure!(metadata.is_file() && !is_path_alias(&metadata),
            "refusing aliased cache transaction lock"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error).context("inspect cache transaction lock"),
    }
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(&lock_path).context("open cache transaction lock")?;
    file.lock().context("acquire OS cache transaction lock")?;
    ensure!(qualified_cache_root(project_root, false)?.as_ref() == Some(&root),
        "cache root changed during transaction admission");
    struct HeldTransaction(PathBuf);
    impl Drop for HeldTransaction {
        fn drop(&mut self) {
            let _ = HELD_CACHE_TRANSACTIONS.try_with(|held| {
                if let Ok(mut held) = held.try_borrow_mut() {
                    held.remove(&self.0);
                }
            });
        }
    }
    HELD_CACHE_TRANSACTIONS.with(|held| held.borrow_mut().insert(lock_path.clone()));
    let _held = HeldTransaction(lock_path);
    // Closing `file` releases the OS lock without invoking a custom destructor.
    operation()
}

/// Durable cache preparation intent persisted so interrupted builds or
/// unavailable warm queues recover cleanly on restart or subsequent admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingCacheIntent {
    pub(crate) schema_version: u16,
    pub(crate) target: ProjectCacheTarget,
    pub(crate) generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) desired_source_hash: Option<HashDigest>,
    pub(crate) config_hash: HashDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) desired_revision_identity: Option<HashDigest>,
    #[serde(default = "pending_by_default")]
    pub(crate) pending: bool,
    pub(crate) created_at_unix_secs: u64,
}

impl PendingCacheIntent {
    pub(crate) fn new(
        target: ProjectCacheTarget,
        generation: u64,
        desired_source_hash: Option<HashDigest>,
        config_hash: HashDigest,
    ) -> Self {
        Self {
            schema_version: PENDING_INTENT_SCHEMA_VERSION,
            target,
            generation,
            desired_source_hash,
            config_hash,
            desired_revision_identity: None,
            pending: true,
            created_at_unix_secs: unix_now_secs(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PENDING_INTENT_SCHEMA_VERSION,
            "unsupported pending cache intent schema version {}",
            self.schema_version
        );
        ensure!(
            self.generation >= 1,
            "pending cache intent generation must be positive"
        );
        Ok(())
    }
}

/// Store for durable pending cache preparation requests.
#[derive(Clone, Debug)]
pub(crate) struct PendingIntentStore {
    project_root: PathBuf,
    directory: PathBuf,
}

impl PendingIntentStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            directory: ProjectStorageLayout::new(project_root).cache_pending_dir(),
        }
    }

    fn intent_path(&self, target: &ProjectCacheTarget) -> PathBuf {
        self.directory
            .join(format!("{}.json", sanitize_scope_key(&target.key())))
    }

    pub(crate) fn record(&self, intent: &PendingCacheIntent) -> Result<PathBuf> {
        with_cache_lock(&self.project_root, "pending", || self.record_locked(intent))
    }

    fn record_locked(&self, intent: &PendingCacheIntent) -> Result<PathBuf> {
        intent.validate()?;
        if let Some(current) = self.load_record(&intent.target)? {
            ensure!(current.generation <= intent.generation, "superseded pending cache intent");
            if current.generation == intent.generation {
                ensure!(current == *intent, "conflicting pending intent at same generation");
                return Ok(self.intent_path(&intent.target));
            }
        }
        self.persist_record(intent)
    }

    fn persist_record(&self, intent: &PendingCacheIntent) -> Result<PathBuf> {
        qualify_cache_directory(&self.directory, true)?;
        let root = qualified_cache_root(&self.project_root, false)?.context("missing cache root")?;
        qualify_cache_entry(&root, &self.directory)?;
        let path = self.intent_path(&intent.target);
        if fs::symlink_metadata(&path).is_ok() { qualify_cache_entry(&root, &path)?; }
        let temporary = self
            .directory
            .join(format!(".pending.{}.tmp", Uuid::new_v4()));
        let bytes =
            serde_json::to_vec_pretty(intent).context("serialize pending cache intent")?;
        write_bytes_atomic_durable(&temporary, &path, &bytes)
            .context("persist pending cache intent")?;
        Ok(path)
    }

    pub(crate) fn load(&self, target: &ProjectCacheTarget) -> Result<Option<PendingCacheIntent>> {
        Ok(self.load_record(target)?.filter(|record| record.pending))
    }

    pub(crate) fn generation_floor(&self, target: &ProjectCacheTarget) -> Result<u64> {
        Ok(self.load_record(target)?.map_or(0, |record| record.generation))
    }

    pub(crate) fn load_record(&self, target: &ProjectCacheTarget) -> Result<Option<PendingCacheIntent>> {
        let Some(root) = qualified_cache_root(&self.project_root, false)? else { return Ok(None); };
        let path = self.intent_path(target);
        match fs::symlink_metadata(&path) {
            Ok(_) => { qualify_cache_entry(&root, &path)?; },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect pending record"),
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read pending cache intent {}", path.display()));
            }
        };
        let intent: PendingCacheIntent = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode pending cache intent {}", path.display()))?;
        intent.validate()?;
        ensure!(
            intent.target == *target,
            "pending cache intent target mismatch"
        );
        Ok(Some(intent))
    }

    pub(crate) fn list_all(&self) -> Result<Vec<PendingCacheIntent>> {
        if !qualify_cache_directory(&self.directory, false)? { return Ok(Vec::new()); }
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "read pending cache intents directory {}",
                        self.directory.display()
                    )
                });
            }
        };
        let mut intents = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            ensure!(qualify_cache_file(&path, false)?, "missing durable cache intent");
            let bytes = fs::read(&path).context("read durable cache intent")?;
            let intent: PendingCacheIntent = serde_json::from_slice(&bytes)
                .context("decode durable cache intent")?;
            intent.validate()?;
            ensure!(self.intent_path(&intent.target) == path, "pending cache intent locator mismatch");
            if intent.pending { intents.push(intent); }
        }
        intents.sort_by_key(|intent| intent.target.key());
        Ok(intents)
    }

    /// Acknowledge only the exact desired generation and identity. The caller
    /// must already have committed a matching current pointer.
    pub(crate) fn acknowledge_if_generation(
        &self,
        target: &ProjectCacheTarget,
        published_generation: u64,
        published_identity: Option<HashDigest>,
    ) -> Result<()> {
        with_cache_lock(&self.project_root, "pending", || {
            let Some(mut current) = self.load(target)? else { return Ok(()); };
            if current.generation != published_generation
                || current.desired_revision_identity.is_none()
                || current.desired_revision_identity != published_identity
            {
                return Ok(());
            }
            current.pending = false;
            self.persist_record(&current)?;
            Ok(())
        })
    }

    pub(crate) fn remove(&self, target: &ProjectCacheTarget) -> Result<()> {
        with_cache_lock(&self.project_root, "pending", || self.remove_locked(target))
    }

    fn remove_locked(&self, target: &ProjectCacheTarget) -> Result<()> {
        let path = self.intent_path(target);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(metadata.is_file() && !is_path_alias(&metadata),
                "refusing aliased pending cache intent"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("inspect pending cache intent"),
        }
        let root = qualified_cache_root(&self.project_root, false)?.context("missing cache root")?;
        qualify_cache_entry(&root, &path)?;
        fs::remove_file(&path).with_context(|| format!("remove pending cache intent {}", path.display()))?;
        sync_parent_best_effort(path.parent());
        Ok(())
    }
}

/// Durable deletion tombstone preventing in-flight builds from resurrecting a
/// deleted cache target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CacheTombstone {
    pub(crate) schema_version: u16,
    pub(crate) target: ProjectCacheTarget,
    pub(crate) retired_generation: u64,
    pub(crate) retired_at_unix_secs: u64,
}

impl CacheTombstone {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == TOMBSTONE_SCHEMA_VERSION,
            "unsupported cache tombstone schema version {}",
            self.schema_version
        );
        ensure!(
            self.retired_generation >= 1,
            "cache tombstone retired generation must be positive"
        );
        Ok(())
    }
}

/// Store for durable target deletion tombstones.
#[derive(Clone, Debug)]
pub(crate) struct TombstoneStore {
    project_root: PathBuf,
    directory: PathBuf,
}

impl TombstoneStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            directory: ProjectStorageLayout::new(project_root).cache_tombstones_dir(),
        }
    }

    fn tombstone_path(&self, target: &ProjectCacheTarget) -> PathBuf {
        self.directory
            .join(format!("{}.json", sanitize_scope_key(&target.key())))
    }

    pub(crate) fn retire(
        &self,
        target: &ProjectCacheTarget,
        retired_generation: u64,
    ) -> Result<CacheTombstone> {
        with_cache_lock(&self.project_root, "retirement", || self.retire_locked(target, retired_generation))
    }

    fn retire_locked(&self, target: &ProjectCacheTarget, retired_generation: u64) -> Result<CacheTombstone> {
        let existing_generation = self
            .load(target)?
            .map_or(0, |tombstone| tombstone.retired_generation);
        let generation = retired_generation.max(existing_generation).max(1);
        let tombstone = CacheTombstone {
            schema_version: TOMBSTONE_SCHEMA_VERSION,
            target: target.clone(),
            retired_generation: generation,
            retired_at_unix_secs: unix_now_secs(),
        };
        tombstone.validate()?;
        qualify_cache_directory(&self.directory, true)?;
        let root = qualified_cache_root(&self.project_root, false)?.context("missing cache root")?;
        qualify_cache_entry(&root, &self.directory)?;
        let path = self.tombstone_path(target);
        if fs::symlink_metadata(&path).is_ok() { qualify_cache_entry(&root, &path)?; }
        let temporary = self
            .directory
            .join(format!(".tombstone.{}.tmp", Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(&tombstone).context("serialize cache tombstone")?;
        write_bytes_atomic_durable(&temporary, &path, &bytes)
            .context("persist cache tombstone")?;
        Ok(tombstone)
    }

    pub(crate) fn load(&self, target: &ProjectCacheTarget) -> Result<Option<CacheTombstone>> {
        let Some(root) = qualified_cache_root(&self.project_root, false)? else { return Ok(None); };
        let path = self.tombstone_path(target);
        match fs::symlink_metadata(&path) {
            Ok(_) => { qualify_cache_entry(&root, &path)?; },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect tombstone"),
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read cache tombstone {}", path.display()));
            }
        };
        let tombstone: CacheTombstone = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode cache tombstone {}", path.display()))?;
        tombstone.validate()?;
        ensure!(
            tombstone.target == *target,
            "cache tombstone target mismatch"
        );
        Ok(Some(tombstone))
    }

    /// Returns `true` if `generation` is at or below a persisted retirement
    /// tombstone and therefore must be rejected to prevent resurrection.
    pub(crate) fn is_retired_at(
        &self,
        target: &ProjectCacheTarget,
        generation: u64,
    ) -> Result<bool> {
        Ok(self
            .load(target)?
            .is_some_and(|tombstone| generation <= tombstone.retired_generation))
    }

    /// Reactivation reads the durable high-water mark without deleting it.
    /// A strictly newer authorized generation supersedes this floor.
    pub(crate) fn reactivation_floor(&self, target: &ProjectCacheTarget) -> Result<u64> {
        Ok(self.load(target)?.map_or(0, |tombstone| tombstone.retired_generation))
    }

}

pub(crate) fn sanitize_scope_key(raw: &str) -> String {
    raw.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Durable Project-wide invalidation barrier for incomplete graph discovery.
const INVALIDATION_BARRIER: &str = "invalidation-barrier.json";

pub(crate) fn cache_invalidation_barrier(project_root: &Path) -> Result<bool> {
    let Some(root) = qualified_cache_root(project_root, false)? else { return Ok(false); };
    let path = root.join(INVALIDATION_BARRIER);
    match fs::symlink_metadata(&path) {
        Ok(_) => { qualify_cache_entry(&root, &path)?; Ok(true) },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("inspect Project cache invalidation barrier"),
    }
}

pub(crate) fn set_cache_invalidation_barrier(project_root: &Path) -> Result<()> {
    with_cache_lock(project_root, "invalidation", || {
        let root = qualified_cache_root(project_root, false)?.context("missing cache root")?;
        let path = root.join(INVALIDATION_BARRIER);
        if fs::symlink_metadata(&path).is_ok() { qualify_cache_entry(&root, &path)?; }
        let temporary = root.join(format!(".invalidation.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic_durable(&temporary, &path, b"{\"requires_reconciliation\":true}")
    })
}

pub(crate) fn clear_cache_invalidation_barrier(project_root: &Path) -> Result<()> {
    with_cache_lock(project_root, "invalidation", || {
        let root = qualified_cache_root(project_root, false)?.context("missing cache root")?;
        let path = root.join(INVALIDATION_BARRIER);
        match fs::symlink_metadata(&path) {
            Ok(_) => { qualify_cache_entry(&root, &path)?; fs::remove_file(&path)?; },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error).context("inspect cache barrier before removal"),
        }
        sync_parent_best_effort(path.parent());
        Ok(())
    })
}
