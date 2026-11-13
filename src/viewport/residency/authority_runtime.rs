use usd_project::SceneId;

use super::super::loader::BoundedLoader;
use super::super::spatial::{CameraCandidateIndex, SceneSpatialPayload};
use super::{ActiveScene, DEFAULT_LOADER_CAPACITY, ResidencyAuthority};

impl ResidencyAuthority {
    pub(crate) fn register_scene_generation(&mut self, scene_id: SceneId, generation: u64) {
        self.scene_generations.insert(scene_id, generation);
    }

    pub(crate) fn generation_for(&self, key: &super::ScenePayloadKey) -> Option<u64> {
        self.records.get(key).map(|record| record.generation)
    }

    pub(crate) fn is_terminal_failure(&self, key: super::ScenePayloadKey, generation: u64) -> bool {
        self.terminal_failures.contains(&(key, generation))
    }

    pub(crate) fn retire(&mut self) {
        self.retire_all_residency();
    }

    pub(crate) fn install_scene(
        &mut self,
        scene_id: SceneId,
        generation: u64,
        payloads: Vec<SceneSpatialPayload>,
    ) {
        self.retire_all_residency();
        self.scene_generations.insert(scene_id, generation);
        self.active_scene = Some(ActiveScene {
            scene_id,
            generation,
            spatial: CameraCandidateIndex::from_payloads(payloads),
        });
        self.last_camera = None;
    }

    fn retire_all_residency(&mut self) {
        let released = self
            .records
            .values_mut()
            .filter_map(|record| record.render_handle.take().map(|handle| handle.id()))
            .collect::<Vec<_>>();
        self.released_render_assets.extend(released);
        self.loader = BoundedLoader::new(DEFAULT_LOADER_CAPACITY);
        self.repair_phases.clear();
        self.terminal_failures.clear();
        self.ready_for_upload.clear();
        self.ready_upload_membership.clear();
        self.warm.clear();
        self.pending_loads.clear();
        self.records.clear();
        self.scene_generations.clear();
        self.cpu_used = 0;
        self.cpu_reserved = 0;
        self.gpu_used = 0;
        self.active_scene = None;
        self.camera_keys.clear();
        self.camera_retry_keys.clear();
        self.last_camera = None;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/authority_runtime_inline_tests.rs"]
mod tests;
