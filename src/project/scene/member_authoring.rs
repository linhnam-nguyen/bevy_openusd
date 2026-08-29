use std::path::Path;

use anyhow::{Context, Result};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{SceneMember, SceneMemberTarget};

use super::{
    REFERENCES_FIELD, SCENE_MEMBERS_PRIM, SCENE_ROOT_PRIM, member_custom_data, scene_member_path,
    scene_path,
};

/// Author one composition placement in the canonical form used by persisted
/// parent Scenes, migration, and the active LiveStage.
pub(crate) fn author_scene_member(
    stage: &Stage,
    project_root: &Path,
    member: &SceneMember,
) -> Result<()> {
    stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str())?
        .set_type_name("Xform")?;
    let (asset_path, referenced_prim) = match member.target {
        SceneMemberTarget::Scene(scene_id) => (scene_path(project_root, scene_id), SCENE_ROOT_PRIM),
        SceneMemberTarget::Model(model_id) => (
            crate::project::model_wrapper::model_wrapper_path(project_root, model_id),
            "ModelRoot",
        ),
    };
    let asset_path = asset_path
        .to_str()
        .context("Project Scene member target path must be valid UTF-8")?
        .to_owned();
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
