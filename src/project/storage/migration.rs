use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use usd_project::{ModelId, ProjectManifestV1, ProjectRoot, SceneId};
use uuid::Uuid;

use super::ProjectStorageLayout;

#[path = "migration_assets.rs"]
mod assets;
#[path = "migration_journal.rs"]
mod journal;
#[path = "migration_publish.rs"]
mod publish;
#[path = "migration_recovery.rs"]
mod recovery;

const TRANSACTIONS_DIRECTORY: &str = ".transactions";
pub(super) const LEGACY_MODEL_MARKER: &str = ".legacy-model.usda";

struct SceneMove {
    id: SceneId,
    old_path: PathBuf,
    final_path: PathBuf,
    staged_path: PathBuf,
    backup_path: PathBuf,
}

struct ModelMove {
    id: ModelId,
    old_dir: PathBuf,
    final_dir: PathBuf,
    old_wrapper: PathBuf,
    staged_wrapper: PathBuf,
    backup_dir: PathBuf,
}

struct DirectoryMove {
    old_dir: PathBuf,
    final_dir: PathBuf,
    backup_dir: PathBuf,
}

struct MigrationPlan {
    transaction_directory: PathBuf,
    scenes: Vec<SceneMove>,
    models: Vec<ModelMove>,
    imports: Vec<DirectoryMove>,
}

/// Upgrade one legacy `.usdhub` Project exactly once.
pub(crate) fn migrate_legacy_project(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    let migrated_manifest = manifest
        .clone()
        .migrate_legacy()
        .context("migrate legacy Project manifest schema")?
        .canonicalized();
    recover_interrupted_migration(project_root, Some(&migrated_manifest))?;
    ensure!(
        !layout.canonical_manifest_present(),
        "Storage v2 manifest unexpectedly exists during legacy migration"
    );
    ensure!(
        layout.legacy_manifest_path().is_file(),
        "legacy Project manifest is missing during migration"
    );
    migrated_manifest
        .validate()
        .context("validate legacy Project migration manifest")?;

    let transaction_directory = layout
        .metadata_dir()
        .join(TRANSACTIONS_DIRECTORY)
        .join(format!("migration-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_directory)
        .context("create Project migration transaction directory")?;
    let plan_result = build_plan(
        project_root,
        &migrated_manifest,
        transaction_directory.clone(),
    );
    let plan = match plan_result {
        Ok(plan) => plan,
        Err(error) => {
            let _ = fs::remove_dir_all(&transaction_directory);
            return Err(error);
        }
    };
    if let Err(error) = publish::write_journal(project_root, &migrated_manifest, &plan) {
        let _ = fs::remove_dir_all(&transaction_directory);
        return Err(error);
    }
    let result = publish::publish_plan(project_root, &migrated_manifest, &plan);
    if let Err(error) = result {
        if let Err(rollback_error) = publish::rollback_plan(&plan) {
            return Err(error.context(format!(
                "rollback legacy Project storage migration failed; preserving transaction: {rollback_error:#}"
            )));
        }
        let canonical_manifest = ProjectStorageLayout::new(project_root).canonical_manifest_path();
        if canonical_manifest.exists() {
            fs::remove_file(&canonical_manifest).with_context(|| {
                format!(
                    "remove incomplete canonical Project manifest {}",
                    canonical_manifest.display()
                )
            })?;
        }
        recovery::verify_rolled_back(project_root, &plan)
            .context("verify rolled-back Project storage migration")?;
        recovery::sync_rolled_back_directories(project_root, &plan)
            .context("sync rolled-back Project storage migration")?;
        recovery::remove_transaction_directory(&plan.transaction_directory)?;
        return Err(error.context("rollback legacy Project storage migration"));
    }
    let journal = journal::read(
        project_root,
        &plan.transaction_directory.join(journal::JOURNAL_FILE),
    )?;
    recovery::finalize_committed_migration(project_root, &plan.transaction_directory, &journal)?;
    recovery::remove_transaction_directory(&plan.transaction_directory)?;
    Ok(())
}

pub(crate) use recovery::recover_interrupted_migration;

fn build_plan(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    transaction_directory: PathBuf,
) -> Result<MigrationPlan> {
    let layout = ProjectStorageLayout::new(project_root);
    let mut asset_map = assets::AssetMap::from_manifest(project_root, manifest)?;
    let mut scenes = Vec::with_capacity(manifest.scenes.len());
    for scene in &manifest.scenes {
        let old_path = layout.legacy_scene_path(scene.id);
        ensure!(
            old_path.is_file(),
            "legacy Project Scene layer is missing: {}",
            old_path.display()
        );
        let final_path = scene_path(&layout, manifest, scene.id, &scene.storage_key);
        ensure_destination_absent(&final_path)?;
        let staged_path = transaction_directory
            .join("staged/scenes")
            .join(format!("{}.usda", scene.id));
        let backup_path = transaction_directory
            .join("backup/scenes")
            .join(format!("{}.usda", scene.id));
        scenes.push(SceneMove {
            id: scene.id,
            old_path,
            final_path,
            staged_path,
            backup_path,
        });
    }

    let mut models = Vec::with_capacity(manifest.models.len());
    for model in &manifest.models {
        let old_wrapper = layout.legacy_model_wrapper_path(model.id);
        let old_dir = old_wrapper
            .parent()
            .context("legacy Model wrapper has no parent directory")?
            .to_owned();
        ensure!(
            old_wrapper.is_file(),
            "legacy Model wrapper is missing: {}",
            old_wrapper.display()
        );
        let final_dir = layout
            .canonical_model_wrapper_path(model)
            .parent()
            .context("canonical Model wrapper has no parent directory")?
            .to_owned();
        ensure_destination_absent(&final_dir)?;
        let staged_wrapper = transaction_directory
            .join("staged/models")
            .join(model.id.to_string())
            .join("model.usda");
        let backup_dir = transaction_directory
            .join("backup/models")
            .join(model.id.to_string());
        asset_map.add_rule(old_dir.clone(), final_dir.clone());
        models.push(ModelMove {
            id: model.id,
            old_dir,
            final_dir,
            old_wrapper,
            staged_wrapper,
            backup_dir,
        });
    }

    let mut imports = Vec::new();
    for (index, (old_dir, final_dir)) in manifest_import_directories(&layout, manifest)
        .into_iter()
        .enumerate()
    {
        if !old_dir.exists() {
            continue;
        }
        ensure!(old_dir.is_dir(), "legacy import path is not a directory");
        ensure_destination_absent(&final_dir)?;
        asset_map.add_rule(old_dir.clone(), final_dir.clone());
        let name = import_backup_name(&old_dir, index);
        imports.push(DirectoryMove {
            old_dir,
            final_dir,
            backup_dir: transaction_directory.join("backup/imports").join(name),
        });
    }

    let plan = MigrationPlan {
        transaction_directory,
        scenes,
        models,
        imports,
    };
    for scene in &plan.scenes {
        assets::author_scene_migration(
            &scene.old_path,
            &scene.staged_path,
            &scene.final_path,
            project_root,
            scene.id,
            &asset_map,
        )?;
    }
    for model in &plan.models {
        assets::author_model_migration(
            &model.old_wrapper,
            &model.staged_wrapper,
            &model.final_dir.join("model.usda"),
            project_root,
            model.id,
            &asset_map,
        )?;
    }
    Ok(plan)
}

fn import_backup_name(old_dir: &Path, index: usize) -> String {
    format!("{}-{index}", old_dir.file_name().unwrap().to_string_lossy())
}

fn scene_path(
    layout: &ProjectStorageLayout,
    manifest: &ProjectManifestV1,
    scene_id: SceneId,
    storage_key: &usd_project::StorageKey,
) -> PathBuf {
    if manifest.root == ProjectRoot::Scene(scene_id) {
        layout.canonical_root_scene_path(storage_key)
    } else {
        layout.canonical_scene_path(storage_key)
    }
}

fn manifest_import_directories(
    layout: &ProjectStorageLayout,
    manifest: &ProjectManifestV1,
) -> Vec<(PathBuf, PathBuf)> {
    let mut directories = Vec::with_capacity(manifest.scenes.len() + manifest.models.len());
    for scene in &manifest.scenes {
        directories.push((
            layout.legacy_scene_import_dir(scene.id),
            layout.canonical_scene_import_dir(scene.id),
        ));
    }
    for model in &manifest.models {
        directories.push((
            layout.legacy_model_import_dir(model.id),
            layout.canonical_model_import_dir(model.id),
        ));
    }
    directories
}

fn ensure_destination_absent(path: &Path) -> Result<()> {
    ensure!(
        !path.exists(),
        "Storage v2 migration destination already exists: {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
pub(super) mod failure_injection {
    use std::sync::atomic::{AtomicU8, Ordering};

    use anyhow::{Result, bail};

    static FAILURE_POINT: AtomicU8 = AtomicU8::new(0);

    pub(super) fn set(point: u8) {
        FAILURE_POINT.store(point, Ordering::SeqCst);
    }

    pub(super) fn maybe(point: u8) -> Result<()> {
        if FAILURE_POINT
            .compare_exchange(point, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            bail!("injected Project migration failure at point {point}");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
