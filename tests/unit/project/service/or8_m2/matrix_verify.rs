//! Graph and placement checks for the OR8 M2 lifecycle matrix.

use std::collections::{HashMap, HashSet};

use openusd::usd::{InitialLoadSet, Stage};
use usd_project::{SceneId, SceneMemberTarget};

use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{read_scene_members, scene_path},
};

use super::matrix::Context;

pub(super) fn verify_scene_placement(
    context: &Context,
    parent: SceneId,
    scene_id: SceneId,
    placement_id: Option<usd_project::SceneMemberId>,
    linked: bool,
) -> Result<(), String> {
    let members = read_scene_members(&scene_path(&context.project_root, parent), parent)
        .map_err(|error| context.trace.failure(format!("read Scene parent: {error}")))?;
    let placement = members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Scene(scene_id))
        .ok_or_else(|| {
            context
                .trace
                .failure(format!("Scene {scene_id} is not placed"))
        })?;
    if placement_id != Some(placement.id) {
        return Err(context.trace.failure("Scene placement identity changed"));
    }
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(
            scene_path(&context.project_root, scene_id)
                .to_string_lossy()
                .as_ref(),
        )
        .map_err(|error| {
            context
                .trace
                .failure(format!("open composed Scene: {error}"))
        })?;
    let marker =
        crate::project::spatial::source_binding_is_linked(&stage.prim("/SceneRoot/Source"))
            .map_err(|error| {
                context
                    .trace
                    .failure(format!("read Scene mode marker: {error}"))
            })?;
    if marker != linked {
        return Err(context.trace.failure("Scene Import/Link marker mismatch"));
    }
    Ok(())
}

pub(super) fn verify_model_placement(
    context: &Context,
    parent: SceneId,
    model_id: usd_project::ModelId,
    placement_id: Option<usd_project::SceneMemberId>,
) -> Result<(), String> {
    let members = read_scene_members(&scene_path(&context.project_root, parent), parent)
        .map_err(|error| context.trace.failure(format!("read Model parent: {error}")))?;
    let placement = members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Model(model_id))
        .ok_or_else(|| {
            context
                .trace
                .failure(format!("Model {model_id} is not placed"))
        })?;
    if placement_id != Some(placement.id) {
        return Err(context.trace.failure("Model placement identity changed"));
    }
    Ok(())
}

pub(super) fn verify_manifest_graph(context: &Context) -> Result<(), String> {
    let manifest = ManifestStore::read_validated(&context.project_root)
        .map_err(|error| format!("read matrix manifest: {error}"))?;
    let mut graph = HashMap::<SceneId, Vec<SceneId>>::new();
    for scene in manifest.scenes() {
        let members = read_scene_members(&scene_path(&context.project_root, scene.id), scene.id)
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
        visit(scene.id, &graph, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn visit(
    scene: SceneId,
    graph: &HashMap<SceneId, Vec<SceneId>>,
    visiting: &mut HashSet<SceneId>,
    visited: &mut HashSet<SceneId>,
) -> Result<(), String> {
    if !visiting.insert(scene) {
        return Err(format!("composition cycle at Scene {scene}"));
    }
    if visited.insert(scene) {
        if let Some(children) = graph.get(&scene) {
            for child in children {
                visit(*child, graph, visiting, visited)?;
            }
        }
    }
    visiting.remove(&scene);
    Ok(())
}
