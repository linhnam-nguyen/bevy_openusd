use super::{DirectoryMove, MigrationPlan, ModelMove, ProjectStorageLayout, SceneMove};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use usd_project::{ModelId, ProjectId, ProjectManifestV1, SceneId};
use uuid::Uuid;
pub(super) const JOURNAL_FILE: &str = "migration.journal";
const JOURNAL_SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) enum MigrationPhase {
    Prepared,
    Publishing,
}
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct MigrationJournalV1 {
    schema_version: u32,
    project_id: String,
    manifest_fingerprint: String,
    phase: MigrationPhase,
    scenes: Vec<JournalSceneMove>,
    models: Vec<JournalModelMove>,
    imports: Vec<JournalDirectoryMove>,
}
#[derive(Debug, Deserialize, Serialize)]
struct JournalSceneMove {
    id: String,
    old_path: String,
    final_path: String,
    staged_path: String,
    backup_path: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct JournalModelMove {
    id: String,
    old_dir: String,
    final_dir: String,
    old_wrapper: String,
    staged_wrapper: String,
    backup_dir: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct JournalDirectoryMove {
    old_dir: String,
    final_dir: String,
    backup_dir: String,
}
pub(super) fn write_new(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    plan: &MigrationPlan,
    journal_path: &Path,
) -> Result<()> {
    ensure!(
        journal_path.parent() == Some(plan.transaction_directory.as_path()),
        "migration journal must be inside its transaction directory"
    );
    let journal = journal_for_plan(project_root, manifest, plan)?;
    write_new_bytes(journal_path, &journal)
}
pub(super) fn set_phase(
    project_root: &Path,
    journal_path: &Path,
    phase: MigrationPhase,
) -> Result<()> {
    let mut journal = read(project_root, journal_path)?;
    journal.phase = phase;
    let bytes = serde_json::to_vec_pretty(&journal).context("serialize migration journal")?;
    let temporary_path =
        journal_path.with_file_name(format!(".{JOURNAL_FILE}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        write_synced_new_file(&temporary_path, &bytes)?;
        fs::rename(&temporary_path, journal_path)
            .with_context(|| format!("publish migration journal {}", journal_path.display()))?;
        sync_transaction_directories(journal_path.parent())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}
pub(super) fn read(project_root: &Path, journal_path: &Path) -> Result<MigrationJournalV1> {
    let bytes = fs::read(journal_path)
        .with_context(|| format!("read migration journal {}", journal_path.display()))?;
    let journal: MigrationJournalV1 = serde_json::from_slice(&bytes)
        .with_context(|| format!("decode migration journal {}", journal_path.display()))?;
    validate(project_root, &journal)?;
    Ok(journal)
}
pub(super) fn plan_from_journal(
    project_root: &Path,
    transaction_directory: PathBuf,
    journal: &MigrationJournalV1,
) -> Result<MigrationPlan> {
    validate(project_root, journal)?;
    let scenes = journal
        .scenes
        .iter()
        .map(|entry| {
            Ok(SceneMove {
                id: SceneId::parse(&entry.id).context("parse journal Scene id")?,
                old_path: project_path(project_root, &entry.old_path)?,
                final_path: project_path(project_root, &entry.final_path)?,
                staged_path: transaction_path(
                    project_root,
                    &transaction_directory,
                    &entry.staged_path,
                )?,
                backup_path: transaction_path(
                    project_root,
                    &transaction_directory,
                    &entry.backup_path,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let models = journal
        .models
        .iter()
        .map(|entry| {
            Ok(ModelMove {
                id: ModelId::parse(&entry.id).context("parse journal Model id")?,
                old_dir: project_path(project_root, &entry.old_dir)?,
                final_dir: project_path(project_root, &entry.final_dir)?,
                old_wrapper: project_path(project_root, &entry.old_wrapper)?,
                staged_wrapper: transaction_path(
                    project_root,
                    &transaction_directory,
                    &entry.staged_wrapper,
                )?,
                backup_dir: transaction_path(
                    project_root,
                    &transaction_directory,
                    &entry.backup_dir,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let imports = journal
        .imports
        .iter()
        .map(|entry| {
            Ok(DirectoryMove {
                old_dir: project_path(project_root, &entry.old_dir)?,
                final_dir: project_path(project_root, &entry.final_dir)?,
                backup_dir: transaction_path(
                    project_root,
                    &transaction_directory,
                    &entry.backup_dir,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MigrationPlan {
        transaction_directory,
        scenes,
        models,
        imports,
    })
}
pub(super) fn validate_canonical_manifest(
    project_root: &Path,
    journal: &MigrationJournalV1,
) -> Result<ProjectManifestV1> {
    let path = ProjectStorageLayout::new(project_root).canonical_manifest_path();
    let bytes = fs::read(&path)
        .with_context(|| format!("read canonical Project manifest {}", path.display()))?;
    let manifest: ProjectManifestV1 = serde_json::from_slice(&bytes)
        .with_context(|| format!("decode canonical Project manifest {}", path.display()))?;
    let migrated = manifest
        .migrate_legacy()
        .context("migrate canonical Project manifest for recovery")?
        .canonicalized();
    migrated
        .validate()
        .context("validate canonical Project manifest")?;
    validate_manifest_identity(&migrated, journal)?;
    Ok(migrated)
}
pub(super) fn validate_legacy_manifest(
    manifest: &ProjectManifestV1,
    journal: &MigrationJournalV1,
) -> Result<()> {
    let migrated = manifest
        .clone()
        .migrate_legacy()
        .context("migrate legacy Project manifest for recovery")?
        .canonicalized();
    migrated
        .validate()
        .context("validate legacy Project manifest")?;
    validate_manifest_identity(&migrated, journal)
}

pub(super) fn validate_manifest_identity(
    manifest: &ProjectManifestV1,
    journal: &MigrationJournalV1,
) -> Result<()> {
    ensure!(
        manifest.project_id.to_string() == journal.project_id,
        "migration journal ProjectId does not match manifest"
    );
    ensure!(
        manifest_fingerprint(manifest)? == journal.manifest_fingerprint,
        "migration journal manifest fingerprint does not match manifest"
    );
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("open directory for synchronization {}", path.display()))?
        .sync_all()
        .with_context(|| format!("sync directory {}", path.display()))
}

fn journal_for_plan(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    plan: &MigrationPlan,
) -> Result<Vec<u8>> {
    let journal = MigrationJournalV1 {
        schema_version: JOURNAL_SCHEMA_VERSION,
        project_id: manifest.project_id.to_string(),
        manifest_fingerprint: manifest_fingerprint(manifest)?,
        phase: MigrationPhase::Prepared,
        scenes: plan
            .scenes
            .iter()
            .map(|entry| {
                Ok(JournalSceneMove {
                    id: entry.id.to_string(),
                    old_path: relative_path(project_root, &entry.old_path)?,
                    final_path: relative_path(project_root, &entry.final_path)?,
                    staged_path: relative_path(project_root, &entry.staged_path)?,
                    backup_path: relative_path(project_root, &entry.backup_path)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        models: plan
            .models
            .iter()
            .map(|entry| {
                Ok(JournalModelMove {
                    id: entry.id.to_string(),
                    old_dir: relative_path(project_root, &entry.old_dir)?,
                    final_dir: relative_path(project_root, &entry.final_dir)?,
                    old_wrapper: relative_path(project_root, &entry.old_wrapper)?,
                    staged_wrapper: relative_path(project_root, &entry.staged_wrapper)?,
                    backup_dir: relative_path(project_root, &entry.backup_dir)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        imports: plan
            .imports
            .iter()
            .map(|entry| {
                Ok(JournalDirectoryMove {
                    old_dir: relative_path(project_root, &entry.old_dir)?,
                    final_dir: relative_path(project_root, &entry.final_dir)?,
                    backup_dir: relative_path(project_root, &entry.backup_dir)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    };
    serde_json::to_vec_pretty(&journal).context("serialize migration journal")
}

fn write_new_bytes(journal_path: &Path, bytes: &[u8]) -> Result<()> {
    write_synced_new_file(journal_path, bytes)?;
    sync_transaction_directories(journal_path.parent())
}

fn write_synced_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create migration journal {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write migration journal {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync migration journal {}", path.display()))
}

fn sync_transaction_directories(transaction_directory: Option<&Path>) -> Result<()> {
    let Some(transaction_directory) = transaction_directory else {
        bail!("migration journal has no transaction directory");
    };
    let mut directory = Some(transaction_directory);
    while let Some(path) = directory {
        sync_directory(path)?;
        directory = path.parent();
    }
    Ok(())
}

fn validate(project_root: &Path, journal: &MigrationJournalV1) -> Result<()> {
    ensure!(
        journal.schema_version == JOURNAL_SCHEMA_VERSION,
        "unsupported migration journal schema {}",
        journal.schema_version
    );
    ProjectId::parse(&journal.project_id).context("validate journal ProjectId")?;
    ensure!(
        journal.manifest_fingerprint.len() == 64
            && journal
                .manifest_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid migration journal manifest fingerprint"
    );
    let mut paths = HashSet::new();
    for entry in &journal.scenes {
        SceneId::parse(&entry.id).context("validate journal Scene id")?;
        validate_project_path(project_root, &entry.old_path, &mut paths)?;
        validate_project_path(project_root, &entry.final_path, &mut paths)?;
        validate_project_path(project_root, &entry.staged_path, &mut paths)?;
        validate_project_path(project_root, &entry.backup_path, &mut paths)?;
    }
    for entry in &journal.models {
        ModelId::parse(&entry.id).context("validate journal Model id")?;
        validate_project_path(project_root, &entry.old_dir, &mut paths)?;
        validate_project_path(project_root, &entry.final_dir, &mut paths)?;
        validate_project_path(project_root, &entry.old_wrapper, &mut paths)?;
        validate_project_path(project_root, &entry.staged_wrapper, &mut paths)?;
        validate_project_path(project_root, &entry.backup_dir, &mut paths)?;
    }
    for entry in &journal.imports {
        validate_project_path(project_root, &entry.old_dir, &mut paths)?;
        validate_project_path(project_root, &entry.final_dir, &mut paths)?;
        validate_project_path(project_root, &entry.backup_dir, &mut paths)?;
    }
    Ok(())
}

fn validate_project_path(
    project_root: &Path,
    relative: &str,
    paths: &mut HashSet<PathBuf>,
) -> Result<()> {
    let path = project_path(project_root, relative)?;
    ensure!(paths.insert(path), "duplicate path in migration journal");
    Ok(())
}

fn project_path(project_root: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = validated_relative_path(relative)?;
    Ok(project_root.join(relative_path))
}

fn transaction_path(
    project_root: &Path,
    transaction_directory: &Path,
    relative: &str,
) -> Result<PathBuf> {
    let path = project_path(project_root, relative)?;
    ensure!(
        path.starts_with(transaction_directory),
        "migration journal transaction path escapes transaction directory"
    );
    Ok(path)
}

fn relative_path(project_root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(project_root)
        .with_context(|| format!("migration path is outside Project root: {}", path.display()))?;
    let validated = validated_relative_path(&relative.to_string_lossy())?;
    Ok(validated.to_string_lossy().into_owned())
}

fn validated_relative_path(relative: &str) -> Result<PathBuf> {
    let path = PathBuf::from(relative);
    ensure!(
        !path.as_os_str().is_empty(),
        "migration journal path is empty"
    );
    ensure!(
        !path.is_absolute(),
        "migration journal path must be relative"
    );
    for component in path.components() {
        ensure!(
            matches!(component, Component::Normal(_)),
            "migration journal path contains an invalid component"
        );
    }
    Ok(path)
}

fn manifest_fingerprint(manifest: &ProjectManifestV1) -> Result<String> {
    let bytes = serde_json::to_vec(&manifest.canonicalized())
        .context("serialize Project manifest for migration fingerprint")?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
