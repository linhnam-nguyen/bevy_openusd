use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use usd_project::{ProjectManifestV1, ProjectRoot};

use super::{
    LEGACY_MODEL_MARKER, MigrationPlan,
    journal::{self, MigrationPhase},
};
use crate::project::{
    catalog::manifest_store::ManifestStore,
    storage::{IgnoreChange, install_managed_ignore},
};

pub(super) fn publish_plan(
    project_root: &std::path::Path,
    manifest: &ProjectManifestV1,
    plan: &MigrationPlan,
) -> Result<()> {
    journal::set_phase(
        project_root,
        &plan.transaction_directory.join(journal::JOURNAL_FILE),
        MigrationPhase::Publishing,
    )
    .context("durably mark Project migration as publishing")?;
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
        maybe_fail_after_model_directory()?;
        let legacy_wrapper = model.final_dir.join("model.usda");
        fs::rename(&legacy_wrapper, model.final_dir.join(LEGACY_MODEL_MARKER))
            .context("stage legacy Model wrapper")?;
        fs::rename(&model.staged_wrapper, &legacy_wrapper)
            .context("publish migrated Model wrapper")?;
    }
    super::durability::sync_and_validate_canonical_targets(project_root, plan)
        .context("durably sync migrated canonical Project assets")?;
    maybe_fail_before_manifest()?;
    let ignore = install_managed_ignore(project_root).context("update Project managed ignore")?;
    if let Err(error) = ManifestStore::write_manifest_atomic(project_root, manifest) {
        restore_ignore(project_root, ignore);
        return Err(error.context("publish migrated Project manifest"));
    }
    maybe_fail_after_manifest()?;
    Ok(())
}

pub(super) fn rollback_plan(plan: &MigrationPlan) -> Result<()> {
    for import in plan.imports.iter().rev() {
        if path_is_present(&import.final_dir) && !path_is_present(&import.backup_dir) {
            fs::rename(&import.final_dir, &import.backup_dir).with_context(|| {
                format!(
                    "rollback migrated import {} to backup",
                    import.final_dir.display()
                )
            })?;
        }
        if path_is_present(&import.backup_dir) {
            if path_is_present(&import.old_dir) {
                bail!(
                    "cannot restore legacy import because destination is occupied: {}",
                    import.old_dir.display()
                );
            }
            fs::rename(&import.backup_dir, &import.old_dir)
                .with_context(|| format!("restore legacy import {}", import.old_dir.display()))?;
        }
    }
    for model in plan.models.iter().rev() {
        let legacy_wrapper = model.final_dir.join("model.usda");
        let marker = model.final_dir.join(LEGACY_MODEL_MARKER);
        if path_is_present(&model.final_dir) {
            if path_is_present(&marker) {
                if path_is_present(&legacy_wrapper) {
                    fs::remove_file(&legacy_wrapper).with_context(|| {
                        format!(
                            "remove published Model wrapper {}",
                            legacy_wrapper.display()
                        )
                    })?;
                }
                fs::rename(&marker, &legacy_wrapper).with_context(|| {
                    format!("restore legacy Model wrapper {}", legacy_wrapper.display())
                })?;
            }
            fs::rename(&model.final_dir, &model.backup_dir).with_context(|| {
                format!(
                    "rollback migrated Model {} to backup",
                    model.final_dir.display()
                )
            })?;
        }
        if path_is_present(&model.backup_dir) {
            if path_is_present(&model.old_dir) {
                bail!(
                    "cannot restore legacy Model because destination is occupied: {}",
                    model.old_dir.display()
                );
            }
            fs::rename(&model.backup_dir, &model.old_dir)
                .with_context(|| format!("restore legacy Model {}", model.old_dir.display()))?;
        }
    }
    for scene in plan.scenes.iter().rev() {
        if path_is_present(&scene.final_path) {
            fs::remove_file(&scene.final_path).with_context(|| {
                format!("remove published Scene {}", scene.final_path.display())
            })?;
        }
        if path_is_present(&scene.backup_path) {
            if path_is_present(&scene.old_path) {
                bail!(
                    "cannot restore legacy Scene because destination is occupied: {}",
                    scene.old_path.display()
                );
            }
            fs::rename(&scene.backup_path, &scene.old_path)
                .with_context(|| format!("restore legacy Scene {}", scene.old_path.display()))?;
        }
    }
    Ok(())
}

fn restore_ignore(project_root: &std::path::Path, change: IgnoreChange) {
    let _ = crate::project::storage::restore_gitignore(project_root, &change);
}

fn is_ordinary_scene(manifest: &ProjectManifestV1, id: usd_project::SceneId) -> bool {
    manifest.root != ProjectRoot::Scene(id)
}

pub(super) fn write_journal(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    plan: &MigrationPlan,
) -> Result<()> {
    journal::write_new(
        project_root,
        manifest,
        plan,
        &plan.transaction_directory.join(journal::JOURNAL_FILE),
    )
}

fn path_is_present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
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

#[cfg(test)]
fn maybe_fail_after_model_directory() -> Result<()> {
    super::failure_injection::maybe(4)
}

#[cfg(not(test))]
fn maybe_fail_after_model_directory() -> Result<()> {
    Ok(())
}

#[cfg(test)]
fn maybe_fail_after_manifest() -> Result<()> {
    super::failure_injection::maybe(3)
}

#[cfg(not(test))]
fn maybe_fail_after_manifest() -> Result<()> {
    Ok(())
}
