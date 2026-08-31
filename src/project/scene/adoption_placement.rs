use anyhow::Result;
use usd_project::{SceneCompositionGraph, SceneId, SceneMember, ScenePlacementTransform};

use super::adoption_support;

/// Propose a placement while preserving the target Scene identity.
pub(crate) fn propose_scene_placement(
    graph: &SceneCompositionGraph,
    parent_scene_id: SceneId,
    target_scene_id: SceneId,
) -> Result<(SceneCompositionGraph, SceneMember)> {
    adoption_support::propose_scene_placement_with_name(
        graph,
        parent_scene_id,
        target_scene_id,
        "",
        ScenePlacementTransform::IDENTITY,
    )
}
