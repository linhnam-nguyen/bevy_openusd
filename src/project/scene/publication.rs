use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use usd_project::{SceneCompositionGraph, SceneId, SceneManifestEntry, SceneMember};
use uuid::Uuid;

use super::{
    author_scene_member_at_path, validate_member_ids, validate_scene_file, validate_scene_targets,
};
use crate::project::{catalog::manifest_store::ManifestStore, storage::ProjectStorageLayout};

pub(crate) fn author_scene_atomic_at_path(
    project_root: &Path,
    final_path: &Path,
    scene_id: SceneId,
    graph: &SceneCompositionGraph,
    members: &[SceneMember],
    protected_root: bool,
    display_name: Option<&str>,
) -> Result<PathBuf> {
    validate_member_ids(members)?;
    validate_scene_targets(graph, scene_id, members)?;
    let scene_directory = final_path
        .parent()
        .context("Project Scene path has no parent directory")?;
    fs::create_dir_all(scene_directory).context("create Project Scene directory")?;

    let temporary_path = scene_directory.join(format!(".{scene_id}.{}.tmp.usda", Uuid::new_v4()));
    let mut temporary_created = false;
    let result = (|| {
        let stage = super::new_scene_stage_with_name_and_protection(
            scene_id,
            display_name,
            protected_root,
        )?;
        for member in members {
            author_scene_member_at_path(&stage, project_root, final_path, member, None)?;
        }
        let temporary_path_string = temporary_path.to_string_lossy().into_owned();
        temporary_created = true;
        stage
            .root_layer()
            .export(&temporary_path_string)
            .context("export temporary Project Scene layer")?;
        validate_scene_file(&temporary_path, scene_id, members)?;
        fs::rename(&temporary_path, final_path).with_context(|| {
            format!(
                "publish temporary Project Scene {} as {}",
                temporary_path.display(),
                final_path.display()
            )
        })?;
        super::sync_parent_best_effort(final_path.parent());
        Ok(final_path.to_path_buf())
    })();

    if result.is_err() && temporary_created {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

pub(crate) fn scene_path(project_root: &Path, scene_id: SceneId) -> PathBuf {
    let layout = ProjectStorageLayout::new(project_root);
    if let Ok(manifest) = ManifestStore::read_validated(project_root)
        && let Some(scene) = manifest.scene(scene_id)
    {
        return layout.readable_scene_path(manifest.raw(), scene);
    }
    if layout.canonical_manifest_present() {
        // An invalid or incomplete canonical manifest must not make a legacy
        // Scene layer authoritative. This best-effort path only surfaces the
        // canonical-data error to the caller.
        return layout
            .canonical_scenes_dir()
            .join(format!("{scene_id}.usda"));
    }
    layout.legacy_scene_path(scene_id)
}

pub(crate) fn scene_path_for_entry(
    project_root: &Path,
    scene: &SceneManifestEntry,
    protected_root: bool,
) -> PathBuf {
    let layout = ProjectStorageLayout::new(project_root);
    if protected_root {
        layout.canonical_root_scene_path(&scene.storage_key)
    } else {
        layout.canonical_scene_path(&scene.storage_key)
    }
}
