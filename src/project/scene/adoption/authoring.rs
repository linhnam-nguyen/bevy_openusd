//! USD layer preparation used by the Scene adoption transaction.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::{vt, sdf, vt::Value, usd::Stage};
use usd_project::{ModelId, SceneId, SceneMember, SceneMemberTarget};

use super::super::authoring;

const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SOURCE_PRIM: &str = "Source";
const REFERENCES_FIELD: &str = "references";

pub(crate) fn author_scene_wrapper_to_path(
    path: &Path,
    project_root: &Path,
    authored_layer_path: &Path,
    scene_id: SceneId,
    source_asset_path: &Path,
    source_metadata_path: &Path,
    source_prims: &[String],
    scene_name: &str,
    spatial: &usd_project::SourceSpatialConvention,
    linked_source: bool,
) -> Result<()> {
    let stage = authoring::new_scene_stage(scene_id)?;
    stage
        .prim("/SceneRoot")?
        .set_metadata("ui:displayName", Value::String(scene_name.to_owned()))?;
    crate::document::stages::metadata::copy_source_time_metadata(source_metadata_path, &stage)?;
    let source_path = format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}");
    let source_asset_path = crate::project::storage::authored_relative_project_asset_path(
        project_root,
        authored_layer_path,
        source_asset_path,
    )?;
    let source_prim = stage
        .define_prim(source_path.as_str())?
        .set_type_name("Xform")?;
    author_source_references(&stage, &source_prim, &source_asset_path, source_prims)?;
    crate::project::assets::spatial::author_referenced_source_normalization(
        &source_prim, spatial, source_metadata_path, source_prims,
    )?;
    crate::project::assets::spatial::author_source_hierarchy_role(&source_prim)?;
    crate::project::assets::spatial::author_source_binding_role(&source_prim, linked_source)?;
    crate::project::scene::timeline::mark_new_adopted_source_timeline(&stage)?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary adopted Scene wrapper")?;
    Ok(())
}

fn author_source_references(
    stage: &Stage,
    source_prim: &openusd::usd::Prim,
    source_asset_path: &str,
    source_prims: &[String],
) -> Result<()> {
    ensure!(
        !source_prims.is_empty(),
        "Scene source has no entrypoint prims"
    );
    if source_prims.len() == 1 {
        source_prim.clone().set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source_asset_path.to_owned(),
                prim_path: sdf::path(&source_prims[0])?,
                ..Default::default()
            }])),
        )?;
        return Ok(());
    }

    for (index, source_path) in source_prims.iter().enumerate() {
        stage
            .define_prim(format!("/SceneRoot/Source/Root_{index}").as_str())?
            .set_type_name("Xform")?
            .set_metadata(
                REFERENCES_FIELD,
                Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                    asset_path: source_asset_path.to_owned(),
                    prim_path: sdf::path(source_path)?,
                    ..Default::default()
                }])),
            )?;
    }
    Ok(())
}

pub(crate) fn validate_scene_wrapper(
    path: &Path,
    scene_id: SceneId,
    spatial: &usd_project::SourceSpatialConvention,
    linked_source: bool,
) -> Result<()> {
    authoring::validate_scene_file(path, scene_id, &[])?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::builder()
        .load(openusd::usd::InitialLoadSet::LoadNone)
        .open(&path_string)
        .context("reopen temporary adopted Scene wrapper")?;
    ensure!(
        stage
            .prim(format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}").as_str())?
            .is_defined()?,
        "adopted Scene wrapper source prim must be defined"
    );
    let source_path = sdf::path(format!("/{SCENE_ROOT_PRIM}/{SOURCE_PRIM}"))?;
    ensure!(
        has_source_reference(&stage, &source_path)?,
        "adopted Scene wrapper must preserve the source as a reference"
    );
    ensure!(
        stage
            .prim(source_path.as_str())?
            .metadata::<Value>("customData")?
            .and_then(|value| match value {
                Value::Dictionary(data) => data
                    .get(crate::project::assets::spatial::USDHUB_HIERARCHY_ROLE_METADATA)
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            })
            .as_deref()
            == Some(crate::project::assets::spatial::USDHUB_TRANSPARENT_SOURCE_ROLE),
        "adopted Scene wrapper source must carry the transparent hierarchy role"
    );
    ensure!(
        crate::project::assets::spatial::source_binding_is_linked(
            &stage.prim(source_path.as_str())?
        )? == linked_source,
        "adopted Scene wrapper linked-source provenance does not match the request"
    );
    validate_source_reference_paths(&stage, &source_path)?;
    let source_prim = stage.prim(source_path.as_str())?;
    ensure!(
        crate::project::assets::spatial::read_source_normalization(&source_prim)?
            == crate::project::assets::spatial::source_normalization_transform(spatial),
        "adopted Scene wrapper spatial normalization does not match the inspected source"
    );
    Ok(())
}

fn has_source_reference(stage: &Stage, source_path: &sdf::Path) -> Result<bool> {
    let source = stage.prim(source_path.as_str())?;
    if stage
        .root_layer()
        .prim(source_path.clone())?
        .is_some_and(|spec| spec.has_field(REFERENCES_FIELD).unwrap_or(false))
    {
        return Ok(true);
    }
    let root_layer = stage.root_layer();
    Ok(source.children()?.into_iter().any(|child| {
        let Ok(path) = sdf::path(child.path().as_str()) else {
            return false;
        };
        root_layer
            .prim(path)
            .ok()
            .flatten()
            .is_some_and(|spec| spec.has_field(REFERENCES_FIELD).unwrap_or(false))
    }))
}

fn validate_source_reference_paths(stage: &Stage, source_path: &sdf::Path) -> Result<()> {
    let mut specs = Vec::new();
    let root_layer = stage.root_layer();
    if let Some(spec) = root_layer.prim(source_path.clone())?
        && spec.has_field(REFERENCES_FIELD)?
    {
        specs.push(spec);
    } else {
        for child in stage.prim(source_path.as_str())?.children()? {
            if let Ok(path) = sdf::path(child.path().as_str())
                && let Some(spec) = root_layer.prim(path)?
                && spec.has_field(REFERENCES_FIELD)?
            {
                specs.push(spec);
            }
        }
    }
    ensure!(
        !specs.is_empty(),
        "adopted Scene wrapper has no source references"
    );
    for spec in specs {
        let Some(Value::ReferenceListOp(references)) = spec.field(REFERENCES_FIELD)? else {
            anyhow::bail!("adopted Scene wrapper source reference list is missing");
        };
        for reference in references.iter() {
            ensure!(
                !reference.asset_path.is_empty() && !Path::new(&reference.asset_path).is_absolute(),
                "adopted Scene wrapper source asset path must be relative"
            );
        }
    }
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
        &crate::project::scene::timeline::TimelineStageOverrides::default(),
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
        &crate::project::scene::timeline::TimelineStageOverrides::default(),
    )
}

pub(crate) fn prepare_parent_layer_with_staged_scene(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
    scene_id: SceneId,
    final_scene_path: &Path,
    staged_scene_path: &Path,
) -> Result<()> {
    let mut overrides = crate::project::scene::timeline::TimelineStageOverrides::default();
    overrides
        .scenes
        .insert(scene_id, staged_scene_path.to_path_buf());
    prepare_parent_layer_inner(
        existing_path,
        temporary_path,
        project_root,
        parent_scene_id,
        members,
        Some((scene_id, final_scene_path)),
        None,
        &overrides,
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
        &crate::project::scene::timeline::TimelineStageOverrides::default(),
    )
}

pub(crate) fn prepare_parent_layer_with_staged_model(
    existing_path: &Path,
    temporary_path: &Path,
    project_root: &Path,
    parent_scene_id: SceneId,
    members: &[SceneMember],
    model_id: ModelId,
    final_model_path: &Path,
    staged_model_path: &Path,
) -> Result<()> {
    let mut overrides = crate::project::scene::timeline::TimelineStageOverrides::default();
    overrides
        .models
        .insert(model_id, staged_model_path.to_path_buf());
    prepare_parent_layer_inner(
        existing_path,
        temporary_path,
        project_root,
        parent_scene_id,
        members,
        None,
        Some((model_id, final_model_path)),
        &overrides,
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
    timeline_overrides: &crate::project::scene::timeline::TimelineStageOverrides,
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
    let mut effective_overrides = timeline_overrides.clone();
    if let Some((scene_id, final_path)) = scene_target {
        effective_overrides
            .scene_targets
            .insert(scene_id, final_path.to_path_buf());
    }
    if let Some((model_id, final_path)) = model_target {
        effective_overrides
            .model_targets
            .insert(model_id, final_path.to_path_buf());
    }
    crate::project::scene::timeline::recompute_scene_stage_timeline(
        &stage,
        project_root,
        existing_path,
        members,
        &effective_overrides,
    )?;
    stage
        .root_layer()
        .export(temporary_path.to_string_lossy().as_ref())
        .context("export temporary parent Scene layer")?;
    Ok(())
}
