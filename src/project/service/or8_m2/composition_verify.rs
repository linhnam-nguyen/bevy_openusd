//! Post-operation graph and placement assertions for M2-C3.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use usd_project::{SceneId, SceneMemberTarget};

use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{read_scene_members, scene_path},
};

pub(super) fn verify_scene_placement(
    project_root: &Path,
    parent: SceneId,
    scene_id: SceneId,
    placement_id: Option<usd_project::SceneMemberId>,
    linked: bool,
) -> Result<(), String> {
    let parent_members = read_scene_members(&scene_path(project_root, parent), parent)
        .map_err(|error| format!("read parent Scene members: {error}"))?;
    let placement = parent_members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Scene(scene_id))
        .ok_or_else(|| format!("Scene {scene_id} is not placed under {parent}"))?;
    if placement_id != Some(placement.id) {
        return Err(format!("Scene {scene_id} placement ID is not stable"));
    }
    let path = scene_path(project_root, scene_id);
    let stage = openusd::usd::Stage::builder()
        .load(openusd::usd::InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .map_err(|error| format!("open generated Scene {scene_id}: {error}"))?;
    if !stage
        .prim("/SceneRoot")
        .is_defined()
        .map_err(|error| error.to_string())?
    {
        return Err(format!("generated Scene {scene_id} has no SceneRoot"));
    }
    let linked_marker =
        crate::project::spatial::source_binding_is_linked(&stage.prim("/SceneRoot/Source"))
            .map_err(|error| format!("read Scene link marker: {error}"))?;
    if linked_marker != linked {
        return Err(format!("Scene {scene_id} Import/Link marker mismatch"));
    }
    if linked && !crate::project::link::binding_path(project_root, scene_id).is_file() {
        return Err(format!("linked Scene {scene_id} has no local binding"));
    }
    Ok(())
}

pub(super) fn verify_model_placement(
    project_root: &Path,
    parent: SceneId,
    model_id: usd_project::ModelId,
    placement_id: Option<usd_project::SceneMemberId>,
) -> Result<(), String> {
    let members = read_scene_members(&scene_path(project_root, parent), parent)
        .map_err(|error| format!("read Model parent members: {error}"))?;
    let placement = members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Model(model_id))
        .ok_or_else(|| format!("Model {model_id} is not placed under {parent}"))?;
    if placement_id != Some(placement.id) {
        return Err(format!("Model {model_id} placement ID is not stable"));
    }
    Ok(())
}

pub(super) fn verify_manifest_graph(project_root: &Path) -> Result<(), String> {
    let manifest = ManifestStore::read_validated(project_root)
        .map_err(|error| format!("read generated Project manifest: {error}"))?;
    let mut graph = HashMap::<SceneId, Vec<SceneId>>::new();
    for scene in manifest.scenes() {
        let members = read_scene_members(&scene_path(project_root, scene.id), scene.id)
            .map_err(|error| format!("read Scene {}: {error}", scene.id))?;
        for member in members {
            if let SceneMemberTarget::Scene(child) = member.target {
                graph.entry(scene.id).or_default().push(child);
            }
        }
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for scene in manifest.scenes() {
        visit_scene(scene.id, &graph, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn visit_scene(
    scene: SceneId,
    graph: &HashMap<SceneId, Vec<SceneId>>,
    visiting: &mut HashSet<SceneId>,
    visited: &mut HashSet<SceneId>,
) -> Result<(), String> {
    if visiting.contains(&scene) {
        return Err(format!("composition cycle detected at Scene {scene}"));
    }
    if !visited.insert(scene) {
        return Ok(());
    }
    visiting.insert(scene);
    if let Some(children) = graph.get(&scene) {
        for child in children {
            visit_scene(*child, graph, visiting, visited)?;
        }
    }
    visiting.remove(&scene);
    Ok(())
}
