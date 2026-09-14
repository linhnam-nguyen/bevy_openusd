use crate::project::cache_contract::SceneCacheActivation;
use bevy::prelude::{Resource, World};
use usd_bevy::LiveStage;
use usd_project::SceneId;

/// Identity-bearing state for the cache-to-canonical visual handoff.
///
/// The render witness copies this state across the Bevy main/render boundary;
/// the main world remains the only owner allowed to retire Scene cache data.
#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingCanonicalVisualHandoff {
    pub(crate) activation_generation: u64,
    pub(crate) live_stage_session_id: u64,
    pub(crate) live_stage_generation: u64,
    pub(crate) scene_id: SceneId,
    pub(crate) scene_cache_generation: u64,
    pub(crate) canonical_ready: bool,
    pub(crate) canonical_frame_rendered: bool,
}

impl PendingCanonicalVisualHandoff {
    pub(crate) fn sync_for_activation(
        world: &mut World,
        preserve_cache: bool,
        activation_generation: u64,
        scene_cache: Option<&SceneCacheActivation>,
    ) {
        if preserve_cache {
            if let Some(scene_cache) = scene_cache {
                Self::begin_for_scene_cache(world, activation_generation, scene_cache);
            }
        } else {
            world.remove_resource::<Self>();
        }
    }

    pub(crate) fn begin_for_scene_cache(
        world: &mut World,
        activation_generation: u64,
        scene_cache: &SceneCacheActivation,
    ) {
        let Some(live_stage) = world.get_non_send::<LiveStage>() else {
            return;
        };
        Self::begin(
            world,
            activation_generation,
            live_stage.stage_identity(),
            scene_cache.descriptor.scene_id,
            scene_cache.descriptor.generation,
        );
    }

    pub(crate) fn begin(
        world: &mut World,
        activation_generation: u64,
        live_stage_identity: (u64, u64),
        scene_id: SceneId,
        scene_cache_generation: u64,
    ) {
        world.insert_resource(Self::new(
            activation_generation,
            live_stage_identity,
            scene_id,
            scene_cache_generation,
        ));
    }

    pub(crate) fn cancel_if_matches(
        world: &mut World,
        scene_id: &SceneId,
        scene_cache_generation: u64,
    ) {
        if world.get_resource::<Self>().is_some_and(|pending| {
            &pending.scene_id == scene_id
                && pending.scene_cache_generation == scene_cache_generation
        }) {
            world.remove_resource::<Self>();
        }
    }

    pub(crate) fn new(
        activation_generation: u64,
        live_stage_identity: (u64, u64),
        scene_id: SceneId,
        scene_cache_generation: u64,
    ) -> Self {
        Self {
            activation_generation,
            live_stage_session_id: live_stage_identity.0,
            live_stage_generation: live_stage_identity.1,
            scene_id,
            scene_cache_generation,
            canonical_ready: false,
            canonical_frame_rendered: false,
        }
    }

    pub(crate) fn matches_stage_identity(&self, identity: (u64, u64)) -> bool {
        (self.live_stage_session_id, self.live_stage_generation) == identity
    }

    pub(crate) fn matches_identity(
        &self,
        activation_generation: u64,
        live_stage_identity: (u64, u64),
        scene_id: &SceneId,
        scene_cache_generation: u64,
    ) -> bool {
        self.activation_generation == activation_generation
            && self.matches_stage_identity(live_stage_identity)
            && &self.scene_id == scene_id
            && self.scene_cache_generation == scene_cache_generation
    }

    pub(crate) fn mark_canonical_ready(&mut self) {
        self.canonical_ready = true;
    }

    pub(crate) fn mark_canonical_frame_rendered(&mut self) {
        self.canonical_frame_rendered = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_includes_stage_generation_and_cache_generation() {
        let scene_id = SceneId::new_v4();
        let handoff = PendingCanonicalVisualHandoff::new(7, (11, 3), scene_id, 19);

        assert!(handoff.matches_identity(7, (11, 3), &scene_id, 19));
        assert!(!handoff.matches_identity(7, (11, 4), &scene_id, 19));
        assert!(!handoff.matches_identity(7, (11, 3), &scene_id, 20));
    }
}
