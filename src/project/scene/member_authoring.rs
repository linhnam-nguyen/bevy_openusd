use std::path::{Path, PathBuf};

use anyhow::Result;
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{SceneMember, SceneMemberTarget};

use super::{REFERENCES_FIELD, SCENE_ROOT_PRIM, member_custom_data, scene_member_path, scene_path};
use crate::project::storage::authored_relative_project_asset_path;

/// Author one composition placement in the canonical form used by persisted
/// parent Scenes, migration, and the active LiveStage.
pub(crate) fn author_scene_member(
    stage: &Stage,
    project_root: &Path,
    member: &SceneMember,
) -> Result<()> {
    let authoring_layer_path = stage_authoring_path(stage, project_root);
    author_scene_member_at_path(stage, project_root, &authoring_layer_path, member, None)
}

pub(crate) fn author_scene_member_at_path(
    stage: &Stage,
    project_root: &Path,
    authoring_layer_path: &Path,
    member: &SceneMember,
    target_path: Option<&Path>,
) -> Result<()> {
    let (asset_path, referenced_prim) = match member.target {
        SceneMemberTarget::Scene(scene_id) => (
            target_path
                .map(Path::to_path_buf)
                .unwrap_or_else(|| scene_path(project_root, scene_id)),
            SCENE_ROOT_PRIM,
        ),
        SceneMemberTarget::Model(model_id) => (
            target_path.map(Path::to_path_buf).unwrap_or_else(|| {
                crate::project::model_wrapper::model_wrapper_path(project_root, model_id)
            }),
            "ModelRoot",
        ),
    };
    let asset_path =
        authored_relative_project_asset_path(project_root, authoring_layer_path, &asset_path)?;
    let reference = sdf::Reference {
        asset_path,
        prim_path: sdf::path(format!("/{referenced_prim}"))?,
        ..Default::default()
    };
    let member_prim = stage
        .define_prim(scene_member_path(member.id).as_str())?
        .set_type_name("Xform")?
        .set_metadata("customData", Value::Dictionary(member_custom_data(member)))?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([reference])),
        )?;
    let member_prim = match member.name.as_ref() {
        Some(name) => member_prim.set_metadata("ui:displayName", Value::String(name.clone()))?,
        None => member_prim,
    };
    super::placement_transform::author_scene_member_transform(&member_prim, member.transform)?;
    Ok(())
}

fn stage_authoring_path(stage: &Stage, project_root: &Path) -> PathBuf {
    let identifier = stage.root_layer().identifier().to_owned();
    let identifier = PathBuf::from(identifier);
    if identifier.is_absolute() && identifier.starts_with(project_root) {
        identifier
    } else {
        // Anonymous LiveStage layers have no filesystem base. The persisted
        // Project layers always call `author_scene_member_at_path`; this
        // fallback keeps the runtime mutation surface relative as well.
        project_root.join("scenes/.live-stage.usda")
    }
}
