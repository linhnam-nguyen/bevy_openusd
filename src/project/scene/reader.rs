use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, bail, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{ModelId, SceneId, SceneMember, SceneMemberId, SceneMemberTarget};

use super::{
    LEGACY_SCENE_SCHEMA_VERSION, MEMBER_ID_METADATA, MEMBER_NAME_METADATA,
    MEMBER_TARGET_ID_METADATA, MEMBER_TARGET_KIND_METADATA, REFERENCES_FIELD, SCENE_MEMBERS_PRIM,
    SCENE_ROOT_PRIM, SCENE_SCHEMA_VERSION, SCHEMA_VERSION_METADATA, placement_transform,
};

pub(crate) fn validate_scene_file(
    path: &Path,
    expected_scene_id: SceneId,
    expected_members: &[SceneMember],
) -> Result<()> {
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("reopen exported Project Scene layer")?;
    let default_prim = stage.default_prim();
    ensure!(
        default_prim
            .as_ref()
            .is_some_and(|token| token.as_str() == SCENE_ROOT_PRIM),
        "Project Scene defaultPrim must be /{SCENE_ROOT_PRIM}"
    );

    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    ensure!(
        root.is_defined()?,
        "Project Scene root prim must be defined"
    );
    let Some(Value::Dictionary(custom_data)) = root.custom_data()? else {
        bail!("Project Scene root prim is missing customData");
    };

    let Some(scene_id_value) = custom_data.get(super::SCENE_ID_METADATA) else {
        bail!(
            "Project Scene root prim is missing {}",
            super::SCENE_ID_METADATA
        );
    };
    let Some(scene_id_text) = scene_id_value.as_str() else {
        bail!(
            "Project Scene {} must be a string",
            super::SCENE_ID_METADATA
        );
    };
    ensure!(
        SceneId::parse(scene_id_text)? == expected_scene_id,
        "Project Scene metadata identity does not match the registry identity"
    );

    let schema_version = match custom_data.get(SCHEMA_VERSION_METADATA) {
        Some(Value::Int(version)) => *version,
        _ => bail!("Project Scene schema version is unsupported or missing"),
    };
    ensure!(
        matches!(
            schema_version,
            LEGACY_SCENE_SCHEMA_VERSION | SCENE_SCHEMA_VERSION
        ),
        "Project Scene schema version is unsupported or missing"
    );
    if schema_version == SCENE_SCHEMA_VERSION {
        ensure!(
            !stage
                .prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str())
                .is_defined()?,
            "Project Scene schema v2 must not contain /{SCENE_MEMBERS_PRIM}"
        );
    }
    for expected_member in expected_members {
        let member_path = if schema_version == LEGACY_SCENE_SCHEMA_VERSION {
            legacy_scene_member_path(expected_member.id)
        } else {
            scene_member_path(expected_member.id)
        };
        let member = stage.prim(member_path.as_str());
        ensure!(
            member.is_defined()?,
            "Project Scene member prim must be defined"
        );
        let member_spec_path = sdf::path(member_path.as_str())?;
        ensure!(
            stage
                .root_layer()
                .prim(&member_spec_path)
                .is_some_and(|spec| spec.has_field(REFERENCES_FIELD)),
            "Project Scene member prim must contain a target reference"
        );
        if schema_version == SCENE_SCHEMA_VERSION {
            validate_relative_member_references(&stage, &member_spec_path)?;
        }
        ensure!(
            read_scene_member_at_path(&stage, &member_path, expected_member.id)?
                == *expected_member,
            "Project Scene member metadata does not match the authored placement"
        );
    }
    Ok(())
}

pub(crate) fn member_custom_data(member: &SceneMember) -> HashMap<String, Value> {
    let (target_kind, target_id) = match member.target {
        SceneMemberTarget::Scene(id) => ("scene", id.to_string()),
        SceneMemberTarget::Model(id) => ("model", id.to_string()),
    };
    let mut data = HashMap::from([
        (
            MEMBER_ID_METADATA.to_owned(),
            Value::String(member.id.to_string()),
        ),
        (
            MEMBER_TARGET_KIND_METADATA.to_owned(),
            Value::String(target_kind.to_owned()),
        ),
        (
            MEMBER_TARGET_ID_METADATA.to_owned(),
            Value::String(target_id),
        ),
    ]);
    data
}

pub(crate) fn read_scene_member(stage: &Stage, member_id: SceneMemberId) -> Result<SceneMember> {
    let member_path = scene_member_path(member_id);
    read_scene_member_at_path(stage, &member_path, member_id)
}

fn read_scene_member_at_path(
    stage: &Stage,
    member_path: &str,
    member_id: SceneMemberId,
) -> Result<SceneMember> {
    let member = stage.prim(member_path);
    let Some(Value::Dictionary(data)) = member.custom_data()? else {
        bail!("Project Scene member is missing customData");
    };
    let encoded_id = metadata_string(&data, MEMBER_ID_METADATA)?;
    ensure!(
        SceneMemberId::parse(encoded_id)? == member_id,
        "Project Scene member metadata identity does not match its prim path"
    );
    let target_kind = metadata_string(&data, MEMBER_TARGET_KIND_METADATA)?;
    let target_id = metadata_string(&data, MEMBER_TARGET_ID_METADATA)?;
    let target = match target_kind {
        "scene" => SceneMemberTarget::Scene(SceneId::parse(target_id)?),
        "model" => SceneMemberTarget::Model(ModelId::parse(target_id)?),
        other => bail!("unsupported Project Scene member target kind {other:?}"),
    };
    let display_name = {
        let root_layer = stage.root_layer();
        let member_spec_path = sdf::path(member_path)?;
        root_layer
            .prim(&member_spec_path)
            .map(|spec| spec.field("ui:displayName"))
            .transpose()?
            .flatten()
    };
    let name = match display_name.or_else(|| data.get(MEMBER_NAME_METADATA).cloned()) {
        Some(value) => Some(
            value
                .as_str()
                .map(str::to_owned)
                .context("Project Scene member name must be a string")?,
        ),
        None => None,
    };
    Ok(SceneMember {
        id: member_id,
        target,
        name,
        transform: placement_transform::read_scene_member_transform(&member)?,
    })
}

/// Read the authored placement records from one validated Project Scene.
pub(crate) fn read_scene_members(
    path: &Path,
    expected_scene_id: SceneId,
) -> Result<Vec<SceneMember>> {
    validate_scene_file(path, expected_scene_id, &[])?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open Project Scene for read projection")?;
    let Some(Value::Dictionary(root_data)) = stage
        .prim(format!("/{SCENE_ROOT_PRIM}").as_str())
        .custom_data()?
    else {
        bail!("Project Scene root prim is missing customData");
    };
    let schema_version = match root_data.get(SCHEMA_VERSION_METADATA) {
        Some(Value::Int(version)) => *version,
        _ => bail!("Project Scene schema version is unsupported or missing"),
    };
    let members_root = if schema_version == LEGACY_SCENE_SCHEMA_VERSION {
        stage.prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str())
    } else {
        stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str())
    };
    if schema_version != LEGACY_SCENE_SCHEMA_VERSION && schema_version != SCENE_SCHEMA_VERSION {
        bail!("Project Scene schema version is unsupported or missing");
    }
    if schema_version == LEGACY_SCENE_SCHEMA_VERSION && !members_root.is_defined()? {
        return Ok(Vec::new());
    }

    let mut members = Vec::new();
    for child in members_root.children()? {
        let Some(Value::Dictionary(data)) = child.custom_data()? else {
            if schema_version == SCENE_SCHEMA_VERSION {
                continue;
            }
            bail!("Project Scene member is missing customData");
        };
        let Some(encoded_member_id) = data.get(MEMBER_ID_METADATA) else {
            if schema_version == SCENE_SCHEMA_VERSION {
                continue;
            }
            bail!("Project Scene member is missing {MEMBER_ID_METADATA}");
        };
        let Some(encoded_member_id) = encoded_member_id.as_str() else {
            bail!("Project Scene member {MEMBER_ID_METADATA} must be a string");
        };
        let member_id = SceneMemberId::parse(encoded_member_id)?;
        members.push(read_scene_member_at_path(
            &stage,
            child.path().as_str(),
            member_id,
        )?);
    }
    members.sort_by_key(|member| member.id);
    Ok(members)
}

fn metadata_string<'a>(data: &'a HashMap<String, Value>, key: &str) -> Result<&'a str> {
    let value = data
        .get(key)
        .with_context(|| format!("Project Scene member is missing {key}"))?;
    value
        .as_str()
        .with_context(|| format!("Project Scene member {key} must be a string"))
}

pub(crate) fn scene_member_path(member_id: SceneMemberId) -> String {
    let path_id = member_id.to_string().replace('-', "");
    format!("/{SCENE_ROOT_PRIM}/Member_{path_id}")
}

pub(crate) fn legacy_scene_member_path(member_id: SceneMemberId) -> String {
    let path_id = member_id.to_string().replace('-', "");
    format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}/Member_{path_id}")
}

pub(crate) fn scene_member_path_for_stage(
    stage: &Stage,
    member_id: SceneMemberId,
) -> Result<String> {
    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    let Some(Value::Dictionary(data)) = root.custom_data()? else {
        bail!("Project Scene root prim is missing customData");
    };
    match data.get(SCHEMA_VERSION_METADATA) {
        Some(Value::Int(LEGACY_SCENE_SCHEMA_VERSION)) => Ok(legacy_scene_member_path(member_id)),
        Some(Value::Int(SCENE_SCHEMA_VERSION)) => Ok(scene_member_path(member_id)),
        _ => bail!("Project Scene schema version is unsupported or missing"),
    }
}

pub(crate) fn prepare_scene_for_direct_members(stage: &Stage) -> Result<()> {
    let legacy_members_path = format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}");
    let legacy_member_paths = stage
        .prim(legacy_members_path.as_str())
        .children()?
        .into_iter()
        .map(|child| child.path().as_str().to_owned())
        .collect::<Vec<_>>();
    if !legacy_member_paths.is_empty() {
        let member_moves = legacy_member_paths
            .iter()
            .map(|member_path| {
                let member_name = member_path
                    .rsplit('/')
                    .next()
                    .expect("legacy Project Scene member path has a name");
                Ok((
                    sdf::path(member_path)?,
                    sdf::path(format!("/{SCENE_ROOT_PRIM}/{member_name}"))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let root_layer_identifier = stage.root_layer().identifier().to_owned();
        stage
            .batch_edit(&[root_layer_identifier.as_str()], |edits| {
                let edit = edits
                    .first_mut()
                    .expect("Project Scene root layer edit exists");
                for (source_path, target_path) in &member_moves {
                    openusd::sdf::copy_spec_within(edit.data_mut(), source_path, target_path)?;
                    edit.remove_spec(source_path)?;
                }
                edit.remove_spec(&sdf::path(&legacy_members_path)?)?;
                Ok(())
            })
            .context("move legacy Project Scene members to direct paths")?;
    }
    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    let mut custom_data = match root.custom_data()? {
        Some(Value::Dictionary(data)) => data,
        _ => HashMap::new(),
    };
    custom_data.insert(
        SCHEMA_VERSION_METADATA.to_owned(),
        Value::Int(SCENE_SCHEMA_VERSION),
    );
    root.set_metadata("customData", Value::Dictionary(custom_data))?;
    Ok(())
}

fn validate_relative_member_references(stage: &Stage, member_spec_path: &sdf::Path) -> Result<()> {
    let root_layer = stage.root_layer();
    let Some(spec) = root_layer.prim(member_spec_path) else {
        bail!("Project Scene member spec is missing");
    };
    let Some(Value::ReferenceListOp(references)) = spec.field(REFERENCES_FIELD)? else {
        bail!("Project Scene member prim must contain a reference list");
    };
    for reference in references.iter() {
        ensure!(
            !reference.asset_path.is_empty(),
            "Project Scene member reference asset path must not be empty"
        );
        ensure!(
            !Path::new(&reference.asset_path).is_absolute(),
            "Project Scene member reference asset path must be relative"
        );
    }
    Ok(())
}
