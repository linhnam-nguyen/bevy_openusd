use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::usd::Stage;
use usd_project::{SceneId, SceneMember};

use super::{read_scene_members, scene_member_path};

/// Replace all authored placements in one Scene with the supplied set.
pub(crate) fn replace_scene_members_atomic(
    path: &Path,
    project_root: &Path,
    expected_scene_id: SceneId,
    members: &[SceneMember],
) -> Result<()> {
    let existing = read_scene_members(path, expected_scene_id)?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open Scene for placement replacement")?;
    for member in existing {
        ensure!(
            stage.remove_prim(scene_member_path(member.id).as_str())?,
            "Project Scene placement was not authored in the parent layer"
        );
    }
    for member in members {
        super::member_authoring::author_scene_member(&stage, project_root, member)?;
    }
    let temporary_path = path.with_file_name(format!(
        ".{}.replace-{}.tmp.usda",
        expected_scene_id,
        uuid::Uuid::new_v4()
    ));
    let temporary_string = temporary_path.to_string_lossy().into_owned();
    let result = (|| {
        stage
            .root_layer()
            .export(&temporary_string)
            .context("export Scene after placement replacement")?;
        super::validate_scene_file(&temporary_path, expected_scene_id, members)?;
        fs::rename(&temporary_path, path).context("publish Scene after placement replacement")?;
        super::sync_parent_best_effort(path.parent());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}
