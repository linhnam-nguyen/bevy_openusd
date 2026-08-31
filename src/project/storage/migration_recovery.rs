use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result, ensure};
use usd_project::ProjectManifestV1;

use super::{
    MigrationPlan, ProjectStorageLayout, TRANSACTIONS_DIRECTORY, durability, journal, publish,
};

pub(crate) fn recover_interrupted_migration(
    project_root: &Path,
    legacy_manifest: Option<&ProjectManifestV1>,
) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    let transactions_directory = layout.metadata_dir().join(TRANSACTIONS_DIRECTORY);
    let entries = match fs::read_dir(&transactions_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "read Project migration transactions {}",
                    transactions_directory.display()
                )
            });
        }
    };
    let mut transactions = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "read Project migration transaction entry {}",
                transactions_directory.display()
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let journal_path = path.join(journal::JOURNAL_FILE);
        match fs::symlink_metadata(&journal_path) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_file(),
                    "Project migration journal is not a regular file: {}",
                    journal_path.display()
                );
                let journal = journal::read(project_root, &journal_path)?;
                transactions.push((path, journal));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "inspect Project migration journal {}",
                        journal_path.display()
                    )
                });
            }
        }
    }
    transactions.sort_by(|left, right| left.0.cmp(&right.0));

    for (transaction_directory, journal) in transactions {
        if layout.canonical_manifest_present() {
            finalize_committed_migration(project_root, &transaction_directory, &journal)?;
            remove_transaction_directory(&transaction_directory)?;
            continue;
        }
        let manifest = legacy_manifest
            .context("cannot recover interrupted Project migration without the legacy manifest")?;
        journal::validate_legacy_manifest(manifest, &journal)?;
        let plan =
            journal::plan_from_journal(project_root, transaction_directory.clone(), &journal)?;
        publish::rollback_plan(&plan).context("rollback interrupted Project storage migration")?;
        verify_rolled_back(project_root, &plan)
            .context("verify interrupted Project storage rollback")?;
        sync_rolled_back_directories(project_root, &plan)
            .context("sync interrupted Project storage rollback")?;
        remove_transaction_directory(&transaction_directory)?;
    }
    Ok(())
}

pub(super) fn finalize_committed_migration(
    project_root: &Path,
    transaction_directory: &Path,
    journal: &journal::MigrationJournalV1,
) -> Result<()> {
    journal::validate_canonical_manifest(project_root, journal)?;
    let plan = journal::plan_from_journal(project_root, transaction_directory.to_owned(), journal)?;
    durability::sync_and_validate_canonical_targets(project_root, &plan)
        .context("validate and durably sync committed canonical Project assets")?;
    let layout = ProjectStorageLayout::new(project_root);
    let legacy_manifest = layout.legacy_manifest_path();
    if path_is_present(&legacy_manifest) {
        let metadata = fs::symlink_metadata(&legacy_manifest)?;
        ensure!(
            metadata.file_type().is_file(),
            "legacy Project manifest is not a regular file: {}",
            legacy_manifest.display()
        );
        let bytes = fs::read(&legacy_manifest).with_context(|| {
            format!(
                "read stale legacy Project manifest {}",
                legacy_manifest.display()
            )
        })?;
        let manifest: ProjectManifestV1 = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "decode stale legacy Project manifest {}",
                legacy_manifest.display()
            )
        })?;
        journal::validate_legacy_manifest(&manifest, journal)?;
        fs::remove_file(&legacy_manifest).with_context(|| {
            format!(
                "remove stale legacy Project manifest {}",
                legacy_manifest.display()
            )
        })?;
    }
    for model in &plan.models {
        let marker = model.final_dir.join(super::LEGACY_MODEL_MARKER);
        if !path_is_present(&marker) {
            continue;
        }
        let metadata = fs::symlink_metadata(&marker)?;
        ensure!(
            metadata.file_type().is_file(),
            "legacy Model marker is not a regular file: {}",
            marker.display()
        );
        fs::remove_file(&marker).with_context(|| {
            format!("remove committed legacy Model marker {}", marker.display())
        })?;
    }
    sync_committed_directories(project_root, &plan)?;
    Ok(())
}

pub(super) fn verify_rolled_back(project_root: &Path, plan: &MigrationPlan) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    ensure!(
        !path_is_present(&layout.canonical_manifest_path()),
        "canonical Project manifest remains after rollback"
    );
    for scene in &plan.scenes {
        ensure!(
            scene.old_path.is_file(),
            "legacy Scene was not restored: {}",
            scene.old_path.display()
        );
        ensure!(
            !path_is_present(&scene.final_path),
            "migrated Scene remains after rollback: {}",
            scene.final_path.display()
        );
    }
    for model in &plan.models {
        ensure!(
            model.old_dir.is_dir() && model.old_wrapper.is_file(),
            "legacy Model was not restored: {}",
            model.old_dir.display()
        );
        ensure!(
            !path_is_present(&model.final_dir),
            "migrated Model remains after rollback: {}",
            model.final_dir.display()
        );
    }
    for import in &plan.imports {
        ensure!(
            import.old_dir.is_dir(),
            "legacy import was not restored: {}",
            import.old_dir.display()
        );
        ensure!(
            !path_is_present(&import.final_dir),
            "migrated import remains after rollback: {}",
            import.final_dir.display()
        );
    }
    Ok(())
}

pub(super) fn remove_transaction_directory(path: &Path) -> Result<()> {
    fs::remove_dir_all(path).with_context(|| {
        format!(
            "remove completed Project migration transaction {}",
            path.display()
        )
    })?;
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    journal::sync_directory(parent)?;
    if fs::read_dir(parent)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
    {
        fs::remove_dir(parent).with_context(|| {
            format!(
                "remove empty Project migration transactions {}",
                parent.display()
            )
        })?;
        if let Some(metadata_directory) = parent.parent() {
            journal::sync_directory(metadata_directory)?;
        }
    }
    Ok(())
}

pub(super) fn sync_rolled_back_directories(
    project_root: &Path,
    plan: &MigrationPlan,
) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    let mut directories = BTreeSet::new();
    directories.insert(layout.root().to_owned());
    directories.insert(layout.metadata_dir().to_owned());
    directories.insert(plan.transaction_directory.clone());
    directories.insert(
        plan.transaction_directory
            .parent()
            .context("migration transaction has no parent")?
            .to_owned(),
    );
    for scene in &plan.scenes {
        directories.insert(
            scene
                .old_path
                .parent()
                .context("legacy Scene has no parent")?
                .to_owned(),
        );
        insert_parent(&mut directories, &scene.final_path);
    }
    for model in &plan.models {
        directories.insert(model.old_dir.clone());
        insert_parent(&mut directories, &model.final_dir);
    }
    for import in &plan.imports {
        directories.insert(import.old_dir.clone());
        insert_parent(&mut directories, &import.final_dir);
    }
    for directory in directories {
        if directory.is_dir() {
            journal::sync_directory(&directory)?;
        }
    }
    Ok(())
}

fn sync_committed_directories(project_root: &Path, plan: &MigrationPlan) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    let mut directories = BTreeSet::new();
    directories.insert(layout.root().to_owned());
    directories.insert(layout.metadata_dir().to_owned());
    directories.insert(plan.transaction_directory.clone());
    directories.insert(
        plan.transaction_directory
            .parent()
            .context("migration transaction has no parent")?
            .to_owned(),
    );
    for scene in &plan.scenes {
        insert_parent(&mut directories, &scene.old_path);
        insert_parent(&mut directories, &scene.final_path);
    }
    for model in &plan.models {
        directories.insert(model.final_dir.clone());
        insert_parent(&mut directories, &model.old_dir);
    }
    for import in &plan.imports {
        insert_parent(&mut directories, &import.old_dir);
        insert_parent(&mut directories, &import.final_dir);
    }
    for directory in directories {
        if directory.is_dir() {
            journal::sync_directory(&directory)?;
        }
    }
    Ok(())
}

fn insert_parent(directories: &mut BTreeSet<std::path::PathBuf>, path: &Path) {
    if let Some(parent) = path.parent() {
        directories.insert(parent.to_owned());
    }
}

fn path_is_present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
