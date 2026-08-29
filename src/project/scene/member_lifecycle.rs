use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::usd::Stage;
use usd_project::{SceneId, SceneMemberId};

use super::{read_scene_members, scene_member_path, sync_parent_best_effort, validate_scene_file};

/// Remove one authored placement while preserving the remaining parent Scene
/// layer and publishing the result atomically.
pub(crate) fn remove_scene_member_atomic(
    path: &Path,
    expected_scene_id: SceneId,
    member_id: SceneMemberId,
) -> Result<()> {
    let members = read_scene_members(path, expected_scene_id)?;
    ensure!(
        members.iter().any(|member| member.id == member_id),
        "Project Scene placement does not exist"
    );
    let remaining = members
        .iter()
        .filter(|member| member.id != member_id)
        .cloned()
        .collect::<Vec<_>>();
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open parent Scene for placement removal")?;
    ensure!(
        stage.remove_prim(scene_member_path(member_id).as_str())?,
        "Project Scene placement was not authored in the parent layer"
    );
    let temporary_path = path.with_file_name(format!(
        ".{}.remove-{}.tmp.usda",
        expected_scene_id, member_id
    ));
    let temporary_string = temporary_path.to_string_lossy().into_owned();
    let result = (|| {
        stage
            .root_layer()
            .export(&temporary_string)
            .context("export parent Scene after placement removal")?;
        validate_scene_file(&temporary_path, expected_scene_id, &remaining)?;
        fs::rename(&temporary_path, path)
            .context("publish parent Scene after placement removal")?;
        sync_parent_best_effort(path.parent());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}
