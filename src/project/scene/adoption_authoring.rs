//! USD layer preparation used by the Scene adoption transaction.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{SceneId, SceneMember};

use super::authoring;

const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SOURCE_PRIM: &str = "Source";
const REFERENCES_FIELD: &str = "references";

pub(crate) fn author_scene_wrapper_to_path(
    path: &Path,
    scene_id: SceneId,
    source: &Path,
    default_prim: &str,
) -> Result<()> {
    let stage = authoring::new_scene_stage(scene_id, &[])?;
    let source_path = format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}");
    stage
        .define_prim(source_path.as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source
                    .to_str()
                    .context("Scene adoption source path must be valid UTF-8")?
                    .to_owned(),
                prim_path: sdf::path(format!("/{default_prim}"))?,
                ..Default::default()
            }])),
        )?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary adopted Scene wrapper")?;
    Ok(())
}

pub(crate) fn validate_scene_wrapper(path: &Path, scene_id: SceneId) -> Result<()> {
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
            .prim(source_path)
            .is_some_and(|spec| spec.has_field(REFERENCES_FIELD)),
        "adopted Scene wrapper must preserve the source as a reference"
    );
    Ok(())
}

pub(crate) fn prepare_parent_layer(
    existing_path: &Path,
    temporary_path: &Path,
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
        stage
            .define_prim(authoring::scene_member_path(member.id).as_str())?
            .set_type_name("Xform")?
            .set_metadata(
                "customData",
                Value::Dictionary(authoring::member_custom_data(member)),
            )?;
    }
    stage
        .root_layer()
        .export(temporary_path.to_string_lossy().as_ref())
        .context("export temporary parent Scene layer")?;
    Ok(())
}
