use usd_project::SceneId;

use super::matrix::Context;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RoundTripSelection {
    pub(super) source_scene: SceneId,
    pub(super) target_scene: SceneId,
    pub(super) source_depth: usize,
    pub(super) target_depth: usize,
}

pub(super) fn scene_depth(context: &Context, scene_id: SceneId) -> usize {
    let mut depth = 0;
    let mut current = scene_id;
    while let Some(parent) = context
        .fixture
        .scenes
        .iter()
        .find(|scene| scene.id == current)
        .and_then(|scene| scene.parent)
    {
        depth += 1;
        current = parent;
    }
    depth
}
