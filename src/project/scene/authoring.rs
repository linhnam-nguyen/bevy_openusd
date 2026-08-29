use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{gf::Matrix4d, sdf::Value, usd::Stage};
use usd_project::{
    ModelId, SceneCompositionGraph, SceneId, SceneMember, SceneMemberId, SceneMemberTarget,
    ScenePlacementTransform,
};
use uuid::Uuid;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const SCENES_DIRECTORY: &str = "scenes";
const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SCENE_MEMBERS_PRIM: &str = "Members";
const SCENE_ID_METADATA: &str = "usdhub:sceneId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";
const MEMBER_ID_METADATA: &str = "usdhub:memberId";
const MEMBER_TARGET_KIND_METADATA: &str = "usdhub:targetKind";
const MEMBER_TARGET_ID_METADATA: &str = "usdhub:targetId";
const MEMBER_NAME_METADATA: &str = "usdhub:name";
const SCENE_SCHEMA_VERSION: i32 = 1;

pub(crate) fn author_scene_atomic(project_root: &Path, scene_id: SceneId) -> Result<PathBuf> {
    author_scene_atomic_with_members(project_root, scene_id, &[])
}

pub(crate) fn author_scene_atomic_with_members(
    project_root: &Path,
    scene_id: SceneId,
    members: &[SceneMember],
) -> Result<PathBuf> {
    let graph = SceneCompositionGraph::default();
    author_scene_atomic_with_graph(project_root, scene_id, &graph, members)
}

pub(crate) fn author_scene_atomic_with_graph(
    project_root: &Path,
    scene_id: SceneId,
    graph: &SceneCompositionGraph,
    members: &[SceneMember],
) -> Result<PathBuf> {
    validate_member_ids(members)?;
    validate_scene_targets(graph, scene_id, members)?;
    let scene_directory = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(SCENES_DIRECTORY);
    fs::create_dir_all(&scene_directory).context("create Project Scene directory")?;

    let final_path = scene_path(project_root, scene_id);
    let temporary_path = scene_directory.join(format!(".{scene_id}.{}.tmp.usda", Uuid::new_v4()));
    let mut temporary_created = false;

    let result = (|| {
        let stage = new_scene_stage(scene_id, members)?;
        let temporary_path_string = temporary_path.to_string_lossy().into_owned();
        temporary_created = true;
        stage
            .root_layer()
            .export(&temporary_path_string)
            .context("export temporary Project Scene layer")?;
        validate_scene_file(&temporary_path, scene_id, members)?;
        fs::rename(&temporary_path, &final_path).with_context(|| {
            format!(
                "publish temporary Project Scene {} as {}",
                temporary_path.display(),
                final_path.display()
            )
        })?;
        sync_parent_best_effort(final_path.parent());
        Ok(final_path.clone())
    })();

    if result.is_err() && temporary_created {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

pub(crate) fn scene_path(project_root: &Path, scene_id: SceneId) -> PathBuf {
    project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(SCENES_DIRECTORY)
        .join(format!("{scene_id}.usda"))
}

pub(crate) fn new_scene_stage(scene_id: SceneId, members: &[SceneMember]) -> Result<Stage> {
    let stage = Stage::builder().in_memory(format!("scene-{scene_id}.usda"))?;
    let custom_data = scene_custom_data(scene_id);

    stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}").as_str())?
        .set_type_name("Xform")?
        .set_metadata("customData", Value::Dictionary(custom_data))?;
    stage.set_default_prim(SCENE_ROOT_PRIM)?;
    crate::project::spatial::author_canonical_stage(&stage)?;
    if !members.is_empty() {
        stage
            .define_prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str())?
            .set_type_name("Xform")?;
    }
    for member in members {
        let member_path = scene_member_path(member.id);
        stage
            .define_prim(member_path.as_str())?
            .set_type_name("Xform")?
            .set_metadata("customData", Value::Dictionary(member_custom_data(member)))?;
        author_scene_member_transform(&stage.prim(member_path.as_str()), member.transform)?;
    }
    Ok(stage)
}

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

    let Some(scene_id_value) = custom_data.get(SCENE_ID_METADATA) else {
        bail!("Project Scene root prim is missing {SCENE_ID_METADATA}");
    };
    let Some(scene_id_text) = scene_id_value.as_str() else {
        bail!("Project Scene {SCENE_ID_METADATA} must be a string");
    };
    ensure!(
        SceneId::parse(scene_id_text)? == expected_scene_id,
        "Project Scene metadata identity does not match the registry identity"
    );

    ensure!(
        custom_data.get(SCHEMA_VERSION_METADATA) == Some(&Value::Int(SCENE_SCHEMA_VERSION)),
        "Project Scene schema version is unsupported or missing"
    );
    for expected_member in expected_members {
        let member_path = scene_member_path(expected_member.id);
        let member = stage.prim(member_path.as_str());
        ensure!(
            member.is_defined()?,
            "Project Scene member prim must be defined"
        );
        ensure!(
            read_scene_member(&stage, expected_member.id)? == *expected_member,
            "Project Scene member metadata does not match the authored placement"
        );
    }
    Ok(())
}

fn scene_custom_data(scene_id: SceneId) -> HashMap<String, Value> {
    HashMap::from([
        (
            SCENE_ID_METADATA.to_owned(),
            Value::String(scene_id.to_string()),
        ),
        (
            SCHEMA_VERSION_METADATA.to_owned(),
            Value::Int(SCENE_SCHEMA_VERSION),
        ),
    ])
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
    if let Some(name) = &member.name {
        data.insert(MEMBER_NAME_METADATA.to_owned(), Value::String(name.clone()));
    }
    data
}

fn read_scene_member(stage: &Stage, member_id: SceneMemberId) -> Result<SceneMember> {
    let member_path = scene_member_path(member_id);
    let member = stage.prim(member_path.as_str());
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
    let name = data
        .get(MEMBER_NAME_METADATA)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .context("Project Scene member name must be a string")
        })
        .transpose()?;
    Ok(SceneMember {
        id: member_id,
        target,
        name,
        transform: read_scene_member_transform(&member)?,
    })
}

pub(crate) fn author_scene_member_transform(
    member_prim: &openusd::usd::Prim,
    transform: ScenePlacementTransform,
) -> Result<()> {
    member_prim
        .create_attribute("xformOp:transform", "matrix4d")?
        .set_custom(false)?
        .set(Value::Matrix4d(Matrix4d(transform.0)))?;
    member_prim
        .create_attribute("xformOpOrder", "token[]")?
        .set_custom(false)?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

fn read_scene_member_transform(
    member_prim: &openusd::usd::Prim,
) -> Result<ScenePlacementTransform> {
    let Some(value) = member_prim.attribute("xformOp:transform").get::<Value>()? else {
        return Ok(ScenePlacementTransform::IDENTITY);
    };
    let Value::Matrix4d(matrix) = value else {
        bail!("Project Scene placement transform must be matrix4d");
    };
    let order = member_prim
        .attribute("xformOpOrder")
        .get::<Value>()?
        .context("Project Scene placement is missing xformOpOrder")?;
    ensure!(
        order == Value::TokenVec(vec!["xformOp:transform".into()]),
        "Project Scene placement must use only xformOp:transform"
    );
    Ok(ScenePlacementTransform(matrix.0))
}

/// Read the authored placement records from one validated Project Scene.
pub(crate) fn read_scene_members(
    path: &Path,
    expected_scene_id: SceneId,
) -> Result<Vec<SceneMember>> {
    validate_scene_file(path, expected_scene_id, &[])?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open Project Scene for read projection")?;
    let members_root = stage.prim(format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}").as_str());
    if !members_root.is_defined()? {
        return Ok(Vec::new());
    }

    let mut members = Vec::new();
    for child in members_root.children()? {
        let Some(Value::Dictionary(data)) = child.custom_data()? else {
            bail!("Project Scene member is missing customData");
        };
        let member_id = SceneMemberId::parse(metadata_string(&data, MEMBER_ID_METADATA)?)?;
        members.push(read_scene_member(&stage, member_id)?);
    }
    members.sort_by_key(|member| member.id);
    Ok(members)
}

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
    format!("/{SCENE_ROOT_PRIM}/{SCENE_MEMBERS_PRIM}/Member_{path_id}")
}

fn validate_member_ids(members: &[SceneMember]) -> Result<()> {
    let unique_ids = members
        .iter()
        .map(|member| member.id)
        .collect::<HashSet<_>>();
    ensure!(
        unique_ids.len() == members.len(),
        "Project Scene members must have unique SceneMemberId values"
    );
    Ok(())
}

fn validate_scene_targets(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    members: &[SceneMember],
) -> Result<()> {
    for member in members {
        if let SceneMemberTarget::Scene(child_scene_id) = &member.target {
            ensure!(
                !graph.would_create_cycle(parent_scene_id, *child_scene_id),
                "Project Scene composition cycle rejected before publication"
            );
        }
    }
    Ok(())
}

fn sync_parent_best_effort(parent: Option<&Path>) {
    let Some(parent) = parent else {
        return;
    };
    let Ok(directory) = File::open(parent) else {
        return;
    };
    let _ = directory.sync_all();
}
#[cfg(test)]
#[path = "authoring_tests.rs"]
mod tests;
