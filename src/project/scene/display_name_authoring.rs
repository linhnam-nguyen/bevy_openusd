use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::{sdf::Value, usd::Stage};
use usd_project::{SceneId, SceneMemberId};
use uuid::Uuid;

use super::{
    MEMBER_NAME_METADATA, read_scene_members, scene_member_path_for_stage, sync_parent_best_effort,
};

/// Update descriptive metadata without changing a managed prim's stable path.
pub(crate) fn update_display_name_atomic(
    path: &Path,
    prim_path: &str,
    display_name: &str,
) -> Result<()> {
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open managed layer for display-name update")?;
    stage
        .prim(prim_path)
        .set_metadata("ui:displayName", Value::String(display_name.to_owned()))?;
    let temporary_path = path.with_file_name(format!(
        ".{}.rename-{}.tmp.usda",
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("managed"),
        Uuid::new_v4()
    ));
    let temporary_string = temporary_path.to_string_lossy().into_owned();
    let result = (|| {
        stage
            .root_layer()
            .export(&temporary_string)
            .context("export managed layer after display-name update")?;
        fs::rename(&temporary_path, path)
            .context("publish managed layer after display-name update")?;
        sync_parent_best_effort(path.parent());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Update one placement's mirrored label while preserving its reference and
/// transform. The custom-data mirror remains compatible with older files.
pub(crate) fn update_member_display_name_atomic(
    path: &Path,
    expected_scene_id: SceneId,
    member_id: SceneMemberId,
    display_name: &str,
) -> Result<()> {
    let members = read_scene_members(path, expected_scene_id)?;
    ensure!(
        members.iter().any(|member| member.id == member_id),
        "Project Scene placement does not exist"
    );
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open parent Scene for display-name update")?;
    let member_path = scene_member_path_for_stage(&stage, member_id)?;
    let member = stage.prim(member_path.as_str());
    member
        .clone()
        .set_metadata("ui:displayName", Value::String(display_name.to_owned()))?;
    let mut custom_data = match member.custom_data()? {
        Some(Value::Dictionary(data)) => data,
        _ => std::collections::HashMap::new(),
    };
    custom_data.insert(
        MEMBER_NAME_METADATA.to_owned(),
        Value::String(display_name.to_owned()),
    );
    member.set_metadata("customData", Value::Dictionary(custom_data))?;
    let temporary_path = path.with_file_name(format!(
        ".{}.rename-{}.tmp.usda",
        expected_scene_id,
        Uuid::new_v4()
    ));
    let temporary_string = temporary_path.to_string_lossy().into_owned();
    let result = (|| {
        stage
            .root_layer()
            .export(&temporary_string)
            .context("export parent Scene after display-name update")?;
        fs::rename(&temporary_path, path)
            .context("publish parent Scene after display-name update")?;
        sync_parent_best_effort(path.parent());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}
