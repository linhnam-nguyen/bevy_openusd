//! Shared Scene graph closures for publication and runtime authority.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use usd_project::{ModelId, ProjectManifestV1, SceneId, SceneMemberTarget};

use crate::project::scene::authoring;

/// Descendants are the complete content subtree exported from a Scene.
pub(crate) fn scene_dependency_closure(
    project_root: &std::path::Path,
    manifest: &ProjectManifestV1,
    root_scene: SceneId,
) -> Result<(HashSet<SceneId>, HashSet<ModelId>)> {
    let members = read_members(project_root, manifest)?;
    ensure!(manifest.scenes.iter().any(|scene| scene.id == root_scene));
    let mut scenes = HashSet::from([root_scene]);
    let mut models = HashSet::new();
    let mut pending = vec![root_scene];
    while let Some(scene_id) = pending.pop() {
        for member in members.get(&scene_id).into_iter().flatten() {
            match member.target {
                SceneMemberTarget::Scene(child) if scenes.insert(child) => pending.push(child),
                SceneMemberTarget::Model(model_id) => {
                    models.insert(model_id);
                }
                SceneMemberTarget::Scene(_) => {}
            }
        }
    }
    validate_models(manifest, &models)?;
    Ok((scenes, models))
}

/// A Scene commit includes the selected Scene, all of its descendants, and
/// every ancestor whose composition references it. Models are then collected
/// from that complete connected closure.
pub(crate) fn scene_commit_closure(
    project_root: &std::path::Path,
    manifest: &ProjectManifestV1,
    root_scene: SceneId,
) -> Result<(HashSet<SceneId>, HashSet<ModelId>)> {
    let members = read_members(project_root, manifest)?;
    ensure!(manifest.scenes.iter().any(|scene| scene.id == root_scene));
    let mut scenes = HashSet::from([root_scene]);
    let mut pending = vec![root_scene];
    while let Some(scene_id) = pending.pop() {
        for member in members.get(&scene_id).into_iter().flatten() {
            if let SceneMemberTarget::Scene(child) = member.target
                && scenes.insert(child)
            {
                pending.push(child);
            }
        }
    }
    let mut parents = HashMap::<SceneId, Vec<SceneId>>::new();
    for (parent, scene_members) in &members {
        for member in scene_members {
            if let SceneMemberTarget::Scene(child) = member.target {
                parents.entry(child).or_default().push(*parent);
            }
        }
    }
    let mut reverse_pending = scenes.iter().copied().collect::<Vec<_>>();
    while let Some(child) = reverse_pending.pop() {
        for parent in parents.get(&child).into_iter().flatten().copied() {
            if scenes.insert(parent) {
                reverse_pending.push(parent);
            }
        }
    }

    let mut models = HashSet::new();
    for scene_id in &scenes {
        for member in members.get(scene_id).into_iter().flatten() {
            if let SceneMemberTarget::Model(model_id) = member.target {
                models.insert(model_id);
            }
        }
    }
    validate_models(manifest, &models)?;
    Ok((scenes, models))
}

fn read_members(
    project_root: &std::path::Path,
    manifest: &ProjectManifestV1,
) -> Result<HashMap<SceneId, Vec<usd_project::SceneMember>>> {
    manifest
        .scenes
        .iter()
        .map(|scene| {
            let path = authoring::scene_path(project_root, scene.id);
            let members = authoring::read_scene_members(&path, scene.id)?;
            Ok((scene.id, members))
        })
        .collect()
}

fn validate_models(manifest: &ProjectManifestV1, models: &HashSet<ModelId>) -> Result<()> {
    for model_id in models {
        ensure!(manifest.models.iter().any(|model| model.id == *model_id));
    }
    Ok(())
}
