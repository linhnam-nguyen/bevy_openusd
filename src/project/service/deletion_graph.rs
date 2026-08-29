//! O(V+E) composition graph planning for definition-level deletion.

use std::collections::{HashMap, HashSet, VecDeque};

use project_protocol::ProjectWriteError;
use usd_project::{ModelId, SceneId, SceneMember, SceneMemberTarget};

use super::delete_error;

#[derive(Clone, Copy)]
pub(super) enum DeleteTarget {
    Scene(SceneId),
    Model(ModelId),
}

pub(super) struct CompositionIndex {
    pub(super) members: HashMap<SceneId, Vec<SceneMember>>,
}

pub(super) struct DeletionPlan {
    pub(super) scenes: HashSet<SceneId>,
    pub(super) models: HashSet<ModelId>,
    pub(super) removed_placements: Vec<(SceneId, usd_project::SceneMemberId)>,
}

pub(super) fn read_composition(
    project_root: &std::path::Path,
    manifest: &usd_project::ValidatedProjectManifest,
) -> Result<CompositionIndex, ProjectWriteError> {
    let mut members = HashMap::new();
    for scene in manifest.scenes() {
        let path = crate::project::scene::authoring::scene_path(project_root, scene.id);
        let scene_members = crate::project::scene::authoring::read_scene_members(&path, scene.id)
            .map_err(|_| delete_error())?;
        members.insert(scene.id, scene_members);
    }
    Ok(CompositionIndex { members })
}

pub(super) fn build_plan(index: &CompositionIndex, target: DeleteTarget) -> DeletionPlan {
    let (scenes, models) = match target {
        DeleteTarget::Model(model_id) => (HashSet::new(), HashSet::from([model_id])),
        DeleteTarget::Scene(scene_id) => {
            let candidates = scene_descendant_closure(index, scene_id);
            let preserved = externally_reachable_scenes(index, &candidates, scene_id);
            let scenes = candidates
                .difference(&preserved)
                .copied()
                .collect::<HashSet<_>>();
            let model_candidates = scenes
                .iter()
                .flat_map(|parent| index.members.get(parent).into_iter().flatten())
                .filter_map(|member| match member.target {
                    SceneMemberTarget::Model(model_id) => Some(model_id),
                    SceneMemberTarget::Scene(_) => None,
                })
                .collect::<HashSet<_>>();
            let models = model_candidates
                .into_iter()
                .filter(|model_id| {
                    !index.members.iter().any(|(parent, members)| {
                        !scenes.contains(parent)
                            && members
                                .iter()
                                .any(|member| member.target == SceneMemberTarget::Model(*model_id))
                    })
                })
                .collect::<HashSet<_>>();
            (scenes, models)
        }
    };
    let removed_placements = index
        .members
        .iter()
        .filter(|(parent, _)| !scenes.contains(parent))
        .flat_map(|(parent, members)| {
            members.iter().filter_map(|member| {
                is_deleted_target_sets(&scenes, &models, &member.target)
                    .then_some((*parent, member.id))
            })
        })
        .collect();
    DeletionPlan {
        scenes,
        models,
        removed_placements,
    }
}

fn scene_descendant_closure(index: &CompositionIndex, root: SceneId) -> HashSet<SceneId> {
    let mut closure = HashSet::new();
    let mut pending = vec![root];
    while let Some(scene_id) = pending.pop() {
        if !closure.insert(scene_id) {
            continue;
        }
        if let Some(members) = index.members.get(&scene_id) {
            pending.extend(members.iter().filter_map(|member| match member.target {
                SceneMemberTarget::Scene(child) => Some(child),
                SceneMemberTarget::Model(_) => None,
            }));
        }
    }
    closure
}

fn externally_reachable_scenes(
    index: &CompositionIndex,
    candidates: &HashSet<SceneId>,
    explicit_root: SceneId,
) -> HashSet<SceneId> {
    let mut preserved = HashSet::new();
    for (parent, members) in &index.members {
        if candidates.contains(parent) {
            continue;
        }
        for member in members {
            if let SceneMemberTarget::Scene(child) = member.target
                && candidates.contains(&child)
            {
                preserved.insert(child);
            }
        }
    }
    preserved.remove(&explicit_root);
    let mut pending = VecDeque::from_iter(preserved.iter().copied());
    while let Some(scene_id) = pending.pop_front() {
        if let Some(members) = index.members.get(&scene_id) {
            for member in members {
                if let SceneMemberTarget::Scene(child) = member.target
                    && candidates.contains(&child)
                    && preserved.insert(child)
                {
                    pending.push_back(child);
                }
            }
        }
    }
    preserved
}

pub(super) fn changed_parents(index: &CompositionIndex, plan: &DeletionPlan) -> Vec<SceneId> {
    index
        .members
        .iter()
        .filter(|(parent, members)| {
            !plan.scenes.contains(parent)
                && members
                    .iter()
                    .any(|member| is_deleted_target(plan, &member.target))
        })
        .map(|(parent, _)| *parent)
        .collect()
}

pub(super) fn is_deleted_target(plan: &DeletionPlan, target: &SceneMemberTarget) -> bool {
    is_deleted_target_sets(&plan.scenes, &plan.models, target)
}

fn is_deleted_target_sets(
    scenes: &HashSet<SceneId>,
    models: &HashSet<ModelId>,
    target: &SceneMemberTarget,
) -> bool {
    match target {
        SceneMemberTarget::Scene(scene_id) => scenes.contains(scene_id),
        SceneMemberTarget::Model(model_id) => models.contains(model_id),
    }
}
