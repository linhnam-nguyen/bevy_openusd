use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{
    sdf::Value,
    usd::{InitialLoadSet, Stage},
};
use usd_project::{ModelId, SceneId};

use super::{MigrationPlan, journal};

/// Validate and durably publish the canonical asset set before the manifest
/// becomes the migration commit marker.
pub(super) fn sync_and_validate_canonical_targets(
    project_root: &Path,
    plan: &MigrationPlan,
) -> Result<()> {
    let mut directories = BTreeSet::new();
    directories.insert(project_root.to_owned());

    for scene in &plan.scenes {
        validate_scene_target(&scene.final_path, scene.id)?;
        sync_file(
            project_root,
            &scene.final_path,
            "canonical Project Scene",
            &mut directories,
        )?;
    }
    for model in &plan.models {
        let wrapper = model.final_dir.join("model.usda");
        validate_model_target(&wrapper, model.id)?;
        sync_file(
            project_root,
            &wrapper,
            "canonical Model wrapper",
            &mut directories,
        )?;
    }
    for import in &plan.imports {
        sync_asset_tree(project_root, &import.final_dir, &mut directories)?;
    }

    sync_directories(directories)
}

fn validate_scene_target(path: &Path, scene_id: SceneId) -> Result<()> {
    ensure_regular_file(path, "canonical Project Scene")?;
    crate::project::scene::authoring::validate_scene_file(path, scene_id, &[])
        .with_context(|| format!("validate canonical Project Scene {}", path.display()))
}

fn validate_model_target(path: &Path, model_id: ModelId) -> Result<()> {
    ensure_regular_file(path, "canonical Model wrapper")?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(&path_string)
        .with_context(|| format!("open canonical Model wrapper {}", path.display()))?;
    ensure!(
        stage
            .default_prim()
            .as_ref()
            .is_some_and(|token| token.as_str() == "ModelRoot"),
        "canonical Model wrapper defaultPrim must be /ModelRoot"
    );
    let root = stage.prim("/ModelRoot");
    ensure!(
        root.is_defined()?,
        "canonical Model wrapper root must be defined"
    );
    let Some(Value::Dictionary(data)) = root.custom_data()? else {
        bail!("canonical Model wrapper root is missing customData");
    };
    ensure!(
        data.get("usdhub:modelId") == Some(&Value::String(model_id.to_string())),
        "canonical Model wrapper identity does not match its journal"
    );
    let source_path = openusd::sdf::path("/ModelRoot/Source")?;
    ensure!(
        stage.prim("/ModelRoot/Source").is_defined()?
            && stage
                .root_layer()
                .prim(&source_path)
                .is_some_and(|spec| spec.has_field("references")),
        "canonical Model wrapper source reference is missing"
    );
    Ok(())
}

fn sync_asset_tree(
    project_root: &Path,
    path: &Path,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect canonical import directory {}", path.display()))?;
    ensure!(
        metadata.file_type().is_dir(),
        "canonical import target is not a directory: {}",
        path.display()
    );
    directories.insert(path.to_owned());
    insert_project_parent_chain(project_root, path, directories);
    for entry in fs::read_dir(path)
        .with_context(|| format!("read canonical import directory {}", path.display()))?
    {
        let entry =
            entry.with_context(|| format!("read canonical import entry in {}", path.display()))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)
            .with_context(|| format!("inspect canonical import entry {}", child.display()))?;
        if metadata.file_type().is_dir() {
            sync_asset_tree(project_root, &child, directories)?;
        } else if metadata.file_type().is_file() {
            sync_file(project_root, &child, "canonical import asset", directories)?;
        } else {
            bail!(
                "canonical import target is not a regular file or directory: {}",
                child.display()
            );
        }
    }
    Ok(())
}

fn ensure_regular_file(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {description} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "{description} is not a regular file: {}",
        path.display()
    );
    Ok(())
}

fn sync_file(
    project_root: &Path,
    path: &Path,
    description: &str,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    File::open(path)
        .with_context(|| format!("open {description} for synchronization {}", path.display()))?
        .sync_all()
        .with_context(|| format!("sync {description} {}", path.display()))?;
    insert_project_parent_chain(project_root, path, directories);
    Ok(())
}

fn insert_project_parent_chain(
    project_root: &Path,
    path: &Path,
    directories: &mut BTreeSet<PathBuf>,
) {
    let mut current = path.parent();
    while let Some(directory) = current {
        directories.insert(directory.to_owned());
        if directory == project_root {
            break;
        }
        current = directory.parent();
    }
}

fn sync_directories(directories: BTreeSet<PathBuf>) -> Result<()> {
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });
    for directory in directories {
        ensure!(
            fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_dir()),
            "canonical synchronization directory is missing: {}",
            directory.display()
        );
        journal::sync_directory(&directory)
            .with_context(|| format!("sync canonical directory {}", directory.display()))?;
    }
    Ok(())
}
