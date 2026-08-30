//! USD layer preparation used by the Scene adoption transaction.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{ModelId, SceneId, SceneMember, SceneMemberTarget};

use super::authoring;

const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SOURCE_PRIM: &str = "Source";
const REFERENCES_FIELD: &str = "references";

pub(crate) fn author_scene_wrapper_to_path(
    path: &Path,
    project_root: &Path,
    authored_layer_path: &Path,
    scene_id: SceneId,
    source_asset_path: &Path,
    default_prim: &str,
    scene_name: &str,
    spatial: &usd_project::SourceSpatialConvention,
) -> Result<()> {
    let stage = authoring::new_scene_stage(scene_id)?;
    stage
        .prim("/SceneRoot")
        .set_metadata("ui:displayName", Value::String(scene_name.to_owned()))?;
    let source_path = format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}");
    let source_asset_path = crate::project::storage::authored_relative_project_asset_path(
        project_root,
        authored_layer_path,
        source_asset_path,
    )?;
    let source_prim = stage
        .define_prim(source_path.as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source_asset_path,
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
    let references = {
        let root_layer = stage.root_layer();
        let source_spec = root_layer
            .prim(&source_path)
            .context("adopted Scene wrapper source spec is missing")?;
        let Some(Value::ReferenceListOp(references)) = source_spec.field(REFERENCES_FIELD)? else {
            anyhow::bail!("adopted Scene wrapper source reference list is missing");
        };
        references
    };
    for reference in references.iter() {
        ensure!(
            !reference.asset_path.is_empty() && !Path::new(&reference.asset_path).is_absolute(),
            "adopted Scene wrapper source asset path must be relative"
        );
    }
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
    prepare_parent_layer_inner(
        existing_path,
        temporary_path,
        project_root,
        parent_scene_id,
        members,
        None,
        None,
    )
}

pub(crate) fn prepare_parent_layer_with_scene_path(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
    scene_id: SceneId,
    scene_path: &Path,
) -> Result<()> {
    prepare_parent_layer_inner(
        existing_path,
        temporary_path,
        project_root,
        parent_scene_id,
        members,
        Some((scene_id, scene_path)),
        None,
    )
}

pub(crate) fn prepare_parent_layer_with_model_path(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
    model_id: ModelId,
    model_path: &Path,
) -> Result<()> {
    prepare_parent_layer_inner(
        existing_path,
        temporary_path,
        project_root,
        parent_scene_id,
        members,
        None,
        Some((model_id, model_path)),
    )
}

fn prepare_parent_layer_inner(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
    scene_target: Option<(SceneId, &Path)>,
    model_target: Option<(ModelId, &Path)>,
) -> Result<()> {
    let stage = if existing_path.exists() {
        let path_string = existing_path.to_string_lossy().into_owned();
        Stage::open(&path_string).context("open existing parent Scene layer")?
    } else {
        authoring::new_scene_stage(parent_scene_id)?
    };
    authoring::prepare_scene_for_direct_members(&stage)?;
    for member in members {
        let target_path = match (&member.target, model_target) {
            (SceneMemberTarget::Scene(member_scene_id), _)
                if scene_target.is_some_and(|(scene_id, _)| *member_scene_id == scene_id) =>
            {
                scene_target.map(|(_, path)| path)
            }
            (SceneMemberTarget::Model(member_model_id), Some((model_id, path)))
                if *member_model_id == model_id =>
            {
                Some(path)
            }
            _ => None,
        };
        authoring::author_scene_member_at_path(
            &stage,
            project_root,
            existing_path,
            member,
            target_path,
        )?;
    }
    stage
        .root_layer()
        .export(temporary_path.to_string_lossy().as_ref())
        .context("export temporary parent Scene layer")?;
    Ok(())
}
