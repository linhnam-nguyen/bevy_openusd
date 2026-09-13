use bevy::prelude::Resource;
use usd_project::SceneId;

/// The cache-first continuation must observe a render schedule boundary before
/// it enters the potentially blocking canonical Stage open. A finite fallback
/// keeps headless or temporarily unavailable render backends from deadlocking
/// activation.
pub(crate) const CACHE_RENDER_FALLBACK_UPDATES: u8 = 3;

#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct CachePresentationGate {
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) presentation_ready: bool,
    pub(crate) rendered_generation: Option<u64>,
    pub(crate) waited_updates: u8,
    pub(crate) fallback_used: bool,
}

impl CachePresentationGate {
    pub(crate) fn waiting(scene_id: SceneId, generation: u64) -> Self {
        Self {
            scene_id,
            generation,
            presentation_ready: true,
            rendered_generation: None,
            waited_updates: 0,
            fallback_used: false,
        }
    }

    pub(crate) fn observe_rendered_frame(&mut self, scene_id: SceneId, generation: u64) {
        if self.presentation_ready && self.scene_id == scene_id && self.generation == generation {
            self.rendered_generation = Some(generation);
        }
    }

    pub(crate) fn can_open_stage(&mut self) -> bool {
        if !self.presentation_ready {
            return false;
        }
        if self.rendered_generation == Some(self.generation) {
            return true;
        }
        self.waited_updates = self.waited_updates.saturating_add(1);
        if self.waited_updates >= CACHE_RENDER_FALLBACK_UPDATES {
            self.fallback_used = true;
            return true;
        }
        false
    }

    #[cfg(test)]
    pub(crate) fn rendered(&self) -> bool {
        self.rendered_generation == Some(self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_frame_opens_immediately_but_missing_renderer_uses_bounded_fallback() {
        let scene = SceneId::new_v4();
        let mut rendered = CachePresentationGate::waiting(scene, 9);
        rendered.observe_rendered_frame(scene, 9);
        assert!(rendered.can_open_stage());
        assert!(!rendered.fallback_used);

        let mut fallback = CachePresentationGate::waiting(scene, 9);
        assert!(!fallback.can_open_stage());
        assert!(!fallback.can_open_stage());
        assert!(fallback.can_open_stage());
        assert!(fallback.fallback_used);
    }

    #[test]
    fn a_stale_rendered_generation_does_not_release_the_gate() {
        let scene = SceneId::new_v4();
        let mut gate = CachePresentationGate::waiting(scene, 4);
        gate.observe_rendered_frame(scene, 3);
        assert!(!gate.rendered());
    }
}
