use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use usd_project::ProjectManifestV1;

use super::{
    DirectoryMove, MIGRATION_JOURNAL, MigrationPlan, ModelMove, ProjectStorageLayout, SceneMove,
    TRANSACTIONS_DIRECTORY, manifest_import_directories, publish, scene_path,
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
        let journal = path.join("migration.journal");
        if fs::read(&journal).ok().as_deref() == Some(MIGRATION_JOURNAL) {
            transactions.push(path);
        }
    }
    transactions.sort();

    for transaction_directory in transactions {
        if layout.canonical_manifest_present() {
            fs::remove_dir_all(&transaction_directory).with_context(|| {
                format!(
                    "remove completed Project migration transaction {}",
                    transaction_directory.display()
                )
            })?;
            continue;
        }
        let manifest = legacy_manifest
            .context("cannot recover interrupted Project migration without the legacy manifest")?;
        let migrated_manifest = manifest
            .clone()
            .migrate_legacy()
            .context("migrate legacy Project manifest for recovery")?
            .canonicalized();
        let plan = recovery_plan(
            project_root,
            &migrated_manifest,
            transaction_directory.clone(),
        );
        publish::rollback_plan(&plan);
        fs::remove_dir_all(&transaction_directory).with_context(|| {
            format!(
                "remove recovered Project migration transaction {}",
                transaction_directory.display()
            )
        })?;
    }
    if fs::read_dir(&transactions_directory)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
    {
        let _ = fs::remove_dir(&transactions_directory);
    }
    Ok(())
}

fn recovery_plan(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    transaction_directory: PathBuf,
) -> MigrationPlan {
    let layout = ProjectStorageLayout::new(project_root);
    let scenes = manifest
        .scenes
        .iter()
        .map(|scene| SceneMove {
            id: scene.id,
            old_path: layout.legacy_scene_path(scene.id),
            final_path: scene_path(&layout, manifest, scene.id, &scene.storage_key),
            staged_path: transaction_directory
                .join("staged/scenes")
                .join(format!("{}.usda", scene.id)),
            backup_path: transaction_directory
                .join("backup/scenes")
                .join(format!("{}.usda", scene.id)),
        })
        .collect();
    let models = manifest
        .models
        .iter()
        .map(|model| ModelMove {
            id: model.id,
            old_dir: layout
                .legacy_model_wrapper_path(model.id)
                .parent()
                .unwrap()
                .to_owned(),
            final_dir: layout
                .canonical_model_wrapper_path(model)
                .parent()
                .unwrap()
                .to_owned(),
            old_wrapper: layout.legacy_model_wrapper_path(model.id),
            staged_wrapper: transaction_directory
                .join("staged/models")
                .join(model.id.to_string())
                .join("model.usda"),
            backup_dir: transaction_directory
                .join("backup/models")
                .join(model.id.to_string()),
        })
        .collect();
    let imports = manifest_import_directories(&layout, manifest)
        .into_iter()
        .enumerate()
        .map(|(index, (old_dir, final_dir))| DirectoryMove {
            old_dir: old_dir.clone(),
            final_dir,
            backup_dir: transaction_directory
                .join("backup/imports")
                .join(super::import_backup_name(&old_dir, index)),
        })
        .collect();
    MigrationPlan {
        transaction_directory,
        scenes,
        models,
        imports,
    }
}
