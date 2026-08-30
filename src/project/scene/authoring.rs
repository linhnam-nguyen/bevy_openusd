use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{
    ModelId, SceneCompositionGraph, SceneId, SceneMember, SceneMemberId, SceneMemberTarget,
};

use super::placement_transform;

#[path = "member_authoring.rs"]
mod member_authoring;
pub(crate) use member_authoring::{author_scene_member, author_scene_member_at_path};
#[path = "reader.rs"]
mod reader;
pub(crate) use reader::{
    legacy_scene_member_path, member_custom_data, prepare_scene_for_direct_members,
    read_scene_member, read_scene_members, scene_member_path, scene_member_path_for_stage,
    validate_scene_file,
};
#[path = "display_name_authoring.rs"]
mod display_name_authoring;
pub(crate) use display_name_authoring::{
    update_display_name_atomic, update_member_display_name_atomic,
};
#[path = "member_lifecycle.rs"]
mod member_lifecycle;
pub(crate) use member_lifecycle::replace_scene_members_atomic;
#[path = "publication.rs"]
mod publication;
pub(crate) use publication::{author_scene_atomic_at_path, scene_path, scene_path_for_entry};

const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SCENE_MEMBERS_PRIM: &str = "Members";
const SCENE_ID_METADATA: &str = "usdhub:sceneId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";
const MEMBER_ID_METADATA: &str = "usdhub:memberId";
const MEMBER_TARGET_KIND_METADATA: &str = "usdhub:targetKind";
const MEMBER_TARGET_ID_METADATA: &str = "usdhub:targetId";
const MEMBER_NAME_METADATA: &str = "usdhub:name";
const REFERENCES_FIELD: &str = "references";
const PROTECTED_ROOT_METADATA: &str = "usdhub:protectedRoot";
const SCENE_SCHEMA_VERSION: i32 = 2;
const LEGACY_SCENE_SCHEMA_VERSION: i32 = 1;

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
    author_scene_atomic_with_graph_and_protection(project_root, scene_id, graph, members, false)
}

pub(crate) fn author_scene_atomic_with_graph_and_protection(
    project_root: &Path,
    scene_id: SceneId,
    graph: &SceneCompositionGraph,
    members: &[SceneMember],
    protected_root: bool,
) -> Result<PathBuf> {
    author_scene_atomic_with_graph_and_protection_and_name(
        project_root,
        scene_id,
        graph,
        members,
        protected_root,
        None,
    )
}

pub(crate) fn author_scene_atomic_with_graph_and_protection_and_name(
    project_root: &Path,
    scene_id: SceneId,
    graph: &SceneCompositionGraph,
    members: &[SceneMember],
    protected_root: bool,
    display_name: Option<&str>,
) -> Result<PathBuf> {
    let final_path = scene_path(project_root, scene_id);
    author_scene_atomic_at_path(
        project_root,
        &final_path,
        scene_id,
        graph,
        members,
        protected_root,
        display_name,
    )
}

pub(crate) fn new_scene_stage(scene_id: SceneId) -> Result<Stage> {
    new_scene_stage_with_protection(scene_id, false)
}

fn new_scene_stage_with_protection(scene_id: SceneId, protected_root: bool) -> Result<Stage> {
    new_scene_stage_with_name_and_protection(scene_id, None, protected_root)
}

pub(crate) fn new_scene_stage_with_name(scene_id: SceneId, display_name: &str) -> Result<Stage> {
    new_scene_stage_with_name_and_protection(scene_id, Some(display_name), false)
}

pub(crate) fn new_scene_stage_with_name_and_protection(
    scene_id: SceneId,
    display_name: Option<&str>,
    protected_root: bool,
) -> Result<Stage> {
    let stage = Stage::builder().in_memory(format!("scene-{scene_id}.usda"))?;
    let custom_data = scene_custom_data(scene_id, protected_root);

    let root = stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}").as_str())?
        .set_type_name("Xform")?
        .set_metadata("customData", Value::Dictionary(custom_data))?;
    if let Some(display_name) = display_name {
        root.set_metadata("ui:displayName", Value::String(display_name.to_owned()))?;
    }
    stage.set_default_prim(SCENE_ROOT_PRIM)?;
    crate::project::spatial::author_canonical_stage(&stage)?;
    Ok(stage)
}

fn scene_custom_data(scene_id: SceneId, protected_root: bool) -> HashMap<String, Value> {
    let mut custom_data = HashMap::from([
        (
            SCENE_ID_METADATA.to_owned(),
            Value::String(scene_id.to_string()),
        ),
        (
            SCHEMA_VERSION_METADATA.to_owned(),
            Value::Int(SCENE_SCHEMA_VERSION),
        ),
    ]);
    if protected_root {
        custom_data.insert(PROTECTED_ROOT_METADATA.to_owned(), Value::Bool(true));
    }
    custom_data
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

pub(super) fn sync_parent_best_effort(parent: Option<&Path>) {
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
