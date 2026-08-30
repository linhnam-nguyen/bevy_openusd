use std::fs;

use anyhow::{Context, Result};
use usd_project::{ProjectManifestV1, ProjectRoot};

use super::{LEGACY_MODEL_MARKER, MigrationPlan, ProjectStorageLayout};
use crate::project::{
    catalog::manifest_store::ManifestStore,
    storage::{IgnoreChange, install_managed_ignore},
};

pub(super) fn publish_plan(
    project_root: &std::path::Path,
    manifest: &ProjectManifestV1,
    plan: &MigrationPlan,
) -> Result<()> {
    for import in &plan.imports {
        fs::create_dir_all(
            import
                .backup_dir
                .parent()
                .expect("import backup has a parent"),
        )?;
        fs::rename(&import.old_dir, &import.backup_dir)
            .with_context(|| format!("stage legacy import {}", import.old_dir.display()))?;
        fs::create_dir_all(import.final_dir.parent().expect("import has a parent"))?;
        fs::rename(&import.backup_dir, &import.final_dir)
            .with_context(|| format!("publish migrated import {}", import.final_dir.display()))?;
    }
    for scene in &plan.scenes {
        fs::create_dir_all(
            scene
                .backup_path
                .parent()
                .expect("Scene backup has a parent"),
        )?;
        fs::rename(&scene.old_path, &scene.backup_path)
            .with_context(|| format!("stage legacy Scene {}", scene.old_path.display()))?;
        fs::create_dir_all(scene.final_path.parent().expect("Scene has a parent"))?;
        fs::rename(&scene.staged_path, &scene.final_path)
            .with_context(|| format!("publish migrated Scene {}", scene.final_path.display()))?;
        if is_ordinary_scene(manifest, scene.id) {
            maybe_fail_after_ordinary_scene()?
        }
    }
    for model in &plan.models {
        fs::create_dir_all(
            model
                .backup_dir
                .parent()
                .expect("Model backup has a parent"),
        )?;
        fs::rename(&model.old_dir, &model.backup_dir)
            .with_context(|| format!("stage legacy Model {}", model.old_dir.display()))?;
        fs::create_dir_all(model.final_dir.parent().expect("Model has a parent"))?;
        fs::rename(&model.backup_dir, &model.final_dir)
            .with_context(|| format!("publish migrated Model {}", model.final_dir.display()))?;
        let legacy_wrapper = model.final_dir.join("model.usda");
        fs::rename(&legacy_wrapper, model.final_dir.join(LEGACY_MODEL_MARKER))
            .context("stage legacy Model wrapper")?;
        fs::rename(&model.staged_wrapper, &legacy_wrapper)
            .context("publish migrated Model wrapper")?;
    }
    maybe_fail_before_manifest()?;
    let ignore = install_managed_ignore(project_root).context("update Project managed ignore")?;
    if let Err(error) = ManifestStore::write_manifest_atomic(project_root, manifest) {
        restore_ignore(project_root, ignore);
        return Err(error.context("publish migrated Project manifest"));
    }
    let legacy_manifest = ProjectStorageLayout::new(project_root).legacy_manifest_path();
    if let Err(error) = fs::remove_file(&legacy_manifest) {
        restore_ignore(project_root, ignore);
        let _ = fs::remove_file(ProjectStorageLayout::new(project_root).canonical_manifest_path());
        return Err(error).with_context(|| {
            format!(
                "remove migrated legacy manifest {}",
                legacy_manifest.display()
            )
        });
    }
    Ok(())
}

pub(super) fn rollback_plan(plan: &MigrationPlan) {
    for import in plan.imports.iter().rev() {
        if import.final_dir.exists() && !import.backup_dir.exists() {
            let _ = fs::rename(&import.final_dir, &import.backup_dir);
        }
        if import.backup_dir.exists() && !import.old_dir.exists() {
            let _ = fs::rename(&import.backup_dir, &import.old_dir);
        }
    }
    for model in plan.models.iter().rev() {
        let legacy_wrapper = model.final_dir.join("model.usda");
        let marker = model.final_dir.join(LEGACY_MODEL_MARKER);
        if model.final_dir.exists() {
            let _ = fs::remove_file(&legacy_wrapper);
            if marker.exists() {
                let _ = fs::rename(&marker, &legacy_wrapper);
            }
            let _ = fs::rename(&model.final_dir, &model.backup_dir);
        }
        if model.backup_dir.exists() && !model.old_dir.exists() {
            let _ = fs::rename(&model.backup_dir, &model.old_dir);
        }
    }
    for scene in plan.scenes.iter().rev() {
        if scene.final_path.exists() {
            let _ = fs::remove_file(&scene.final_path);
        }
        if scene.backup_path.exists() && !scene.old_path.exists() {
            let _ = fs::rename(&scene.backup_path, &scene.old_path);
        }
    }
}

fn restore_ignore(project_root: &std::path::Path, change: IgnoreChange) {
    let _ = crate::project::storage::restore_gitignore(project_root, &change);
}

fn is_ordinary_scene(manifest: &ProjectManifestV1, id: usd_project::SceneId) -> bool {
    manifest.root != ProjectRoot::Scene(id)
}

pub(super) fn write_journal(plan: &MigrationPlan) -> Result<()> {
    let journal = plan.transaction_directory.join("migration.journal");
    fs::write(
        &journal,
        b"USDHub Storage v2 migration in progress\nmanifest-published-last\n",
    )
    .with_context(|| format!("write migration journal {}", journal.display()))
}

#[cfg(test)]
fn maybe_fail_after_ordinary_scene() -> Result<()> {
    super::failure_injection::maybe(1)
}

#[cfg(not(test))]
fn maybe_fail_after_ordinary_scene() -> Result<()> {
    Ok(())
}

#[cfg(test)]
fn maybe_fail_before_manifest() -> Result<()> {
    super::failure_injection::maybe(2)
}

#[cfg(not(test))]
fn maybe_fail_before_manifest() -> Result<()> {
    Ok(())
}
