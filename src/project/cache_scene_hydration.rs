//! Scene-V3 material/texture hydration and animation-payload residency.

use std::collections::HashMap;

use super::cache_scene_payload::SceneAnimationBlob;

#[derive(bevy::ecs::resource::Resource, Clone, Debug, Default)]
pub(crate) struct SceneAnimationPayloads {
    pub(crate) scene_id: Option<usd_project::SceneId>,
    pub(crate) generation: Option<u64>,
    pub(crate) by_address:
        HashMap<super::cache_contract::SceneCacheAddress, SceneAnimationBlob>,
}
