//! USD layer preparation used by the Scene adoption transaction.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{SceneId, SceneMember, SceneMemberTarget};

use super::authoring;

const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SOURCE_PRIM: &str = "Source";
const REFERENCES_FIELD: &str = "references";

pub(crate) fn author_scene_wrapper_to_path(
    path: &Path,
    scene_id: SceneId,
    source_asset_path: &str,
    default_prim: &str,
    spatial: &usd_project::SourceSpatialConvention,
) -> Result<()> {
    let stage = authoring::new_scene_stage(scene_id, &[])?;
    let source_path = format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}");
    let source_prim = stage
        .define_prim(source_path.as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source_asset_path.to_owned(),
                prim_path: sdf::path(format!("/{default_prim}"))?,
                ..Default::default()
            }])),
        )?;
    crate::project::spatial::author_source_normalization(&source_prim, spatial)?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary adopted Scene wrapper")?;
    Ok(())
}

pub(crate) fn validate_scene_wrapper(
    path: &Path,
    scene_id: SceneId,
    spatial: &usd_project::SourceSpatialConvention,
) -> Result<()> {
    authoring::validate_scene_file(path, scene_id, &[])?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::builder()
        .load(openusd::usd::InitialLoadSet::LoadNone)
        .open(&path_string)
        .context("reopen temporary adopted Scene wrapper")?;
    ensure!(
        stage
            .prim(format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}").as_str())
            .is_defined()?,
        "adopted Scene wrapper source prim must be defined"
    );
    let source_path = sdf::path(format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}"))?;
    ensure!(
        stage
            .root_layer()
            .prim(&source_path)
            .is_some_and(|spec| spec.has_field(REFERENCES_FIELD)),
        "adopted Scene wrapper must preserve the source as a reference"
    );
    let source_prim = stage.prim(source_path.as_str());
    ensure!(
        crate::project::spatial::read_source_normalization(&source_prim)?
            == crate::project::spatial::source_normalization_transform(spatial),
        "adopted Scene wrapper spatial normalization does not match the inspected source"
    );
    Ok(())
}

pub(crate) fn prepare_parent_layer(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
) -> Result<()> {
    let stage = if existing_path.exists() {
        let path_string = existing_path.to_string_lossy().into_owned();
        Stage::open(&path_string).context("open existing parent Scene layer")?
    } else {
        authoring::new_scene_stage(parent_scene_id, &[])?
    };
    stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}/Members").as_str())?
        .set_type_name("Xform")?;
    for member in members {
        author_scene_member(&stage, project_root, member)?;
    }
    stage
        .root_layer()
        .export(temporary_path.to_string_lossy().as_ref())
        .context("export temporary parent Scene layer")?;
    Ok(())
}

/// Author one composition placement in the same form used by canonical
/// parent layers and the active LiveStage.
pub(crate) fn author_scene_member(
    stage: &Stage,
    project_root: &Path,
    member: &SceneMember,
) -> Result<()> {
    let (asset_path, referenced_prim) = match member.target {
        SceneMemberTarget::Scene(scene_id) => (
            authoring::scene_path(project_root, scene_id),
            SCENE_ROOT_PRIM,
        ),
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
        .define_prim(authoring::scene_member_path(member.id).as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            "customData",
            Value::Dictionary(authoring::member_custom_data(member)),
        )?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([reference])),
        )?;
    authoring::author_scene_member_transform(&member_prim, member.transform)?;
    Ok(())
}
