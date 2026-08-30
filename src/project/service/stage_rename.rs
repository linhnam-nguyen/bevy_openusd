use project_protocol::ProjectWriteTarget;

use super::filesystem_error;

pub(super) fn apply_rename_to_live_stage(
    live: &usd_bevy::LiveStage,
    target: &ProjectWriteTarget,
    name: &str,
) -> Result<(), project_protocol::ProjectWriteError> {
    let mut renamed = false;
    match target {
        ProjectWriteTarget::Project(_) | ProjectWriteTarget::Scene(_) => {
            let root = live.stage.prim("/SceneRoot");
            if root.is_defined().map_err(|_| filesystem_error())? {
                if matches!(target, ProjectWriteTarget::Project(_)) {
                    root.clone()
                        .set_metadata(
                            "ui:displayName",
                            openusd::sdf::Value::String(name.to_owned()),
                        )
                        .map_err(|_| filesystem_error())?;
                    renamed = true;
                }
                if let ProjectWriteTarget::Scene(scene_id) = target {
                    let scene_id_text = scene_id.to_string();
                    let Some(openusd::sdf::Value::Dictionary(data)) =
                        root.custom_data().map_err(|_| filesystem_error())?
                    else {
                        return Ok(());
                    };
                    if data
                        .get("usdhub:sceneId")
                        .and_then(openusd::sdf::Value::as_str)
                        == Some(scene_id_text.as_str())
                    {
                        root.set_metadata(
                            "ui:displayName",
                            openusd::sdf::Value::String(name.to_owned()),
                        )
                        .map_err(|_| filesystem_error())?;
                        renamed = true;
                    }
                }
            }
        }
        ProjectWriteTarget::Model(_) => {
            let root = live.stage.prim("/ModelRoot");
            if root.is_defined().map_err(|_| filesystem_error())? {
                root.set_metadata(
                    "ui:displayName",
                    openusd::sdf::Value::String(name.to_owned()),
                )
                .map_err(|_| filesystem_error())?;
                renamed = true;
            }
        }
    }

    let target_metadata = match target {
        ProjectWriteTarget::Scene(id) => Some(("scene", id.to_string())),
        ProjectWriteTarget::Model(id) => Some(("model", id.to_string())),
        ProjectWriteTarget::Project(_) => None,
    };
    let direct_members_root = live.stage.prim("/SceneRoot");
    let legacy_members_root = live.stage.prim("/SceneRoot/Members");
    let mut member_roots = vec![direct_members_root];
    if legacy_members_root
        .is_defined()
        .map_err(|_| filesystem_error())?
    {
        member_roots.push(legacy_members_root);
    }
    if let Some((expected_kind, expected_id)) = target_metadata.as_ref() {
        for members_root in member_roots {
            if !members_root.is_defined().map_err(|_| filesystem_error())? {
                continue;
            }
            for member in members_root.children().map_err(|_| filesystem_error())? {
                let Some(openusd::sdf::Value::Dictionary(data)) =
                    member.custom_data().map_err(|_| filesystem_error())?
                else {
                    continue;
                };
                if data
                    .get("usdhub:targetKind")
                    .and_then(openusd::sdf::Value::as_str)
                    != Some(expected_kind)
                    || data
                        .get("usdhub:targetId")
                        .and_then(openusd::sdf::Value::as_str)
                        != Some(expected_id.as_str())
                {
                    continue;
                }
                member
                    .clone()
                    .set_metadata(
                        "ui:displayName",
                        openusd::sdf::Value::String(name.to_owned()),
                    )
                    .map_err(|_| filesystem_error())?;
                let mut custom_data = data;
                custom_data.insert(
                    "usdhub:name".to_owned(),
                    openusd::sdf::Value::String(name.to_owned()),
                );
                member
                    .set_metadata("customData", openusd::sdf::Value::Dictionary(custom_data))
                    .map_err(|_| filesystem_error())?;
                renamed = true;
            }
        }
    }
    let _ = renamed;
    Ok(())
}
