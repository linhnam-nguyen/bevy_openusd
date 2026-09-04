//! Exact USDZ package discovery for one Project activation target.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use project_protocol::ProjectStageTarget;
use usd_project::{ProjectManifestV1, ProjectRoot};

use crate::project::{model_wrapper::model_wrapper_path, storage::ProjectStorageLayout};

/// Return only packages belonging to the prepared Project target.
///
/// Project import directories are the authoritative package closure. This
/// avoids forcing composed prim traversal on the activation path while keeping
/// unrelated sibling imports out of the active texture cache.
pub(crate) fn for_target(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    target: &ProjectStageTarget,
    stage_path: &Path,
) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    let mut scene_ids = Vec::new();
    let mut model_ids = Vec::new();
    match target {
        ProjectStageTarget::Scene(scene_id) => scene_ids.push(*scene_id),
        ProjectStageTarget::Model(model_id) => model_ids.push(*model_id),
        ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(scene_id)) => scene_ids.push(*scene_id),
        ProjectStageTarget::ProjectRoot(ProjectRoot::Model(model_id)) => model_ids.push(*model_id),
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty) => {}
    }

    for scene_id in scene_ids {
        let (scenes, models) = crate::project::service::scene_closure::scene_dependency_closure(
            project_root,
            manifest,
            scene_id,
        )?;
        for scene in scenes {
            directories
                .push(ProjectStorageLayout::new(project_root).readable_scene_import_dir(scene));
        }
        model_ids.extend(models);
    }
    for model_id in model_ids {
        let wrapper = model_wrapper_path(project_root, model_id);
        if let Some(parent) = wrapper.parent() {
            directories.push(parent.to_path_buf());
        }
    }

    let mut paths = HashSet::new();
    if is_usdz(stage_path) {
        paths.insert(canonical_path(stage_path));
    }
    for directory in directories {
        collect_usdz_files(&directory, &mut paths)?;
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn collect_usdz_files(directory: &Path, paths: &mut HashSet<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read Project import directory {}", directory.display()));
        }
    };
    for entry in entries {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_usdz_files(&path, paths)?;
        } else if metadata.is_file() && is_usdz(&path) {
            paths.insert(canonical_path(&path));
        }
    }
    Ok(())
}

fn is_usdz(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
}

fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use usd_project::ProjectId;

    #[test]
    fn empty_project_has_no_import_packages() {
        let root = tempfile::tempdir().expect("temporary Project root");
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Archive closure",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        assert!(
            for_target(
                root.path(),
                &manifest,
                &ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
                &root.path().join("scene.usda"),
            )
            .expect("archive closure")
            .is_empty()
        );
    }
}
