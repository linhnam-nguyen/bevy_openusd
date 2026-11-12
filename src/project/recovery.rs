//! Git-neutral local recovery checkpoints for the in-memory working stage.
//!
//! Recovery files are deliberately separate from Git and semantic cache
//! storage. A checkpoint contains an exported root layer plus metadata that
//! identifies the live-stage session, revision, source stage, and content
//! digest. The digest prevents restoring a stage file paired with stale or
//! partially-written metadata after a crash.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use bevy::prelude::Resource;
use openusd::usd::{PrimPredicate, Stage};
use serde::{Deserialize, Serialize};
use usd_bevy::LiveStage;

use super::storage::ProjectStorageLayout;

pub(crate) const RECOVERY_FORMAT_VERSION: u32 = 1;
const WORKING_STAGE_FILE: &str = "working.usda";
const METADATA_FILE: &str = "metadata.json";

/// Metadata stored beside one local working-stage checkpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RecoveryMetadata {
    pub(crate) format_version: u32,
    pub(crate) session_id: u64,
    pub(crate) live_revision: u64,
    pub(crate) source_stage: String,
    pub(crate) base_revision: Option<String>,
    pub(crate) stage_digest: String,
    pub(crate) created_at_unix_ms: u64,
}

/// Owned recovery input prepared on the LiveStage owner thread.
#[derive(Debug)]
pub(crate) struct RecoveryCheckpointWork {
    pub(crate) project_root: PathBuf,
    pub(crate) session_id: u64,
    pub(crate) live_revision: u64,
    pub(crate) source_stage: String,
    pub(crate) base_revision: Option<String>,
    pub(crate) stage_bytes: Vec<u8>,
}

/// A validated checkpoint ready to become a new `LiveStage`.
pub(crate) struct RecoveredCheckpoint {
    pub(crate) metadata: RecoveryMetadata,
    pub(crate) stage: Stage,
}

/// Filesystem location for one runtime session's scratch recovery state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryStore {
    directory: PathBuf,
    session_id: u64,
}

/// Runtime configuration for optional scratch recovery.
#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoverySettings {
    pub(crate) project_root: PathBuf,
}

impl Default for RecoverySettings {
    fn default() -> Self {
        let project_root = std::env::var_os("USDHUB_RECOVERY_ROOT")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self { project_root }
    }
}

impl RecoveryStore {
    /// Creates a store rooted at `<project>/.usdhub/recovery/<runtime-session>`.
    pub(crate) fn new(project_root: impl AsRef<Path>, session_id: u64) -> Result<Self> {
        let project_root = project_root.as_ref();
        if project_root.as_os_str().is_empty() {
            bail!("recovery project root must not be empty")
        }
        if session_id == 0 {
            bail!("recovery session id must be non-zero")
        }

        Ok(Self {
            directory: ProjectStorageLayout::new(project_root)
                .recovery_dir()
                .join(format!("session-{session_id}")),
            session_id,
        })
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn stage_path(&self) -> PathBuf {
        self.directory.join(WORKING_STAGE_FILE)
    }

    pub(crate) fn metadata_path(&self) -> PathBuf {
        self.directory.join(METADATA_FILE)
    }

    /// Export the live root layer and metadata as one recoverable checkpoint.
    ///
    /// Both files are replaced atomically. The metadata stores a digest of the
    /// exported stage, so a crash between the two replacements is detected
    /// instead of silently restoring mismatched files.
    pub(crate) fn write_checkpoint(
        &self,
        live_stage: &LiveStage,
        source_stage: &Path,
        base_revision: Option<&str>,
    ) -> Result<RecoveryMetadata> {
        if source_stage.as_os_str().is_empty() {
            bail!("recovery source stage path must not be empty")
        }

        let stage_bytes = usd_bevy::authoring::export_stage_string(&live_stage.stage)
            .context("serializing recovery stage")?
            .into_bytes();
        self.write_checkpoint_bytes(
            &stage_bytes,
            live_stage.current_revision().0,
            source_stage,
            base_revision,
        )
    }

    /// Persist an already-exported stage without touching the owner-thread USD stage.
    pub(crate) fn write_checkpoint_bytes(
        &self,
        stage_bytes: &[u8],
        live_revision: u64,
        source_stage: &Path,
        base_revision: Option<&str>,
    ) -> Result<RecoveryMetadata> {
        if source_stage.as_os_str().is_empty() {
            bail!("recovery source stage path must not be empty")
        }

        fs::create_dir_all(&self.directory)
            .with_context(|| format!("creating recovery directory {}", self.directory.display()))?;

        let stage_path = self.stage_path();
        let stage_temp = temporary_path(&stage_path);
        let metadata_path = self.metadata_path();
        let metadata_temp = temporary_path(&metadata_path);

        let result = (|| {
            let mut stage_file = File::create(&stage_temp)
                .with_context(|| format!("creating recovery stage {}", stage_temp.display()))?;
            stage_file
                .write_all(stage_bytes)
                .with_context(|| format!("writing recovery stage {}", stage_temp.display()))?;
            stage_file
                .sync_all()
                .with_context(|| format!("syncing recovery stage {}", stage_temp.display()))?;
            let stage_digest = blake3::hash(stage_bytes).to_hex().to_string();

            let metadata = RecoveryMetadata {
                format_version: RECOVERY_FORMAT_VERSION,
                session_id: self.session_id,
                live_revision,
                source_stage: source_stage.to_string_lossy().into_owned(),
                base_revision: base_revision.map(str::to_owned),
                stage_digest,
                created_at_unix_ms: unix_time_ms(),
            };
            let metadata_bytes =
                serde_json::to_vec_pretty(&metadata).context("serializing recovery metadata")?;

            fs::rename(&stage_temp, &stage_path)
                .with_context(|| format!("installing recovery stage {}", stage_path.display()))?;
            write_atomic_file(&metadata_temp, &metadata_path, &metadata_bytes)?;
            sync_directory(&self.directory);
            Ok(metadata)
        })();

        if result.is_err() {
            let _ = fs::remove_file(&stage_temp);
            let _ = fs::remove_file(&metadata_temp);
        }
        result
    }

    /// Read and validate checkpoint metadata without opening the USD stage.
    pub(crate) fn inspect(&self) -> Result<Option<RecoveryMetadata>> {
        let stage_exists = self.stage_path().exists();
        let metadata_exists = self.metadata_path().exists();
        match (stage_exists, metadata_exists) {
            (false, false) => return Ok(None),
            (true, false) | (false, true) => {
                bail!("recovery checkpoint is incomplete")
            }
            (true, true) => {}
        }

        let metadata_bytes = fs::read(self.metadata_path()).context("reading recovery metadata")?;
        let metadata: RecoveryMetadata =
            serde_json::from_slice(&metadata_bytes).context("decoding recovery metadata")?;
        self.validate_metadata(&metadata)?;

        let stage_bytes = fs::read(self.stage_path()).context("reading recovery stage")?;
        let actual_digest = blake3::hash(&stage_bytes).to_hex().to_string();
        if actual_digest != metadata.stage_digest {
            bail!(
                "recovery stage digest mismatch: metadata {}, actual {}",
                metadata.stage_digest,
                actual_digest
            )
        }

        Ok(Some(metadata))
    }

    /// Open a validated checkpoint as a fresh OpenUSD stage.
    pub(crate) fn restore(&self) -> Result<Option<RecoveredCheckpoint>> {
        let Some(metadata) = self.inspect()? else {
            return Ok(None);
        };
        let stage_path = self.stage_path();
        let stage_path_string = stage_path.to_string_lossy().into_owned();
        let stage = Stage::open(&stage_path_string)
            .with_context(|| format!("opening recovery stage {}", stage_path.display()))?;
        stage
            .traverse(PrimPredicate::DEFAULT, |_| {})
            .context("validating recovered USD stage")?;
        Ok(Some(RecoveredCheckpoint { metadata, stage }))
    }

    /// Remove this session's scratch state after it is no longer recoverable.
    pub(crate) fn clear(&self) -> Result<bool> {
        if !self.directory.exists() {
            return Ok(false);
        }
        fs::remove_dir_all(&self.directory)
            .with_context(|| format!("removing recovery directory {}", self.directory.display()))?;
        Ok(true)
    }

    fn validate_metadata(&self, metadata: &RecoveryMetadata) -> Result<()> {
        if metadata.format_version != RECOVERY_FORMAT_VERSION {
            bail!(
                "unsupported recovery format version {}",
                metadata.format_version
            )
        }
        if metadata.session_id != self.session_id {
            bail!(
                "recovery session mismatch: expected {}, found {}",
                self.session_id,
                metadata.session_id
            )
        }
        if metadata.stage_digest.is_empty() {
            bail!("recovery metadata has no stage digest")
        }
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recovery");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!(
        ".{file_name}.{}.{}.{}",
        std::process::id(),
        nonce,
        extension
    ))
}

fn write_atomic_file(temp_path: &Path, final_path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(temp_path)
        .with_context(|| format!("creating recovery temp file {}", temp_path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing recovery temp file {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing recovery temp file {}", temp_path.display()))?;
    fs::rename(temp_path, final_path)
        .with_context(|| format!("installing recovery metadata {}", final_path.display()))?;
    Ok(())
}

fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../tests/unit/project/recovery_tests.rs"]
mod tests;
