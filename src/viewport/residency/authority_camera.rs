use super::super::camera::CameraAdmission;
use super::super::spatial::CameraRegion;
use super::{ResidencyAuthority, ResidencyReason};

impl ResidencyAuthority {
    pub(super) fn clear_camera_reasons(&mut self) {
        let Some(generation) = self.active_scene.as_ref().map(|scene| scene.generation) else {
            self.camera_keys.clear();
            return;
        };
        for key in self.camera_keys.clone() {
            self.remove_reason(key, ResidencyReason::CameraNear, generation);
        }
        self.camera_keys.clear();
    }

    fn request_camera_near(
        &mut self,
        key: super::ScenePayloadKey,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
    ) -> bool {
        let accepted = self.request_reason(
            key,
            ResidencyReason::CameraNear,
            generation,
            cpu_bytes,
            gpu_bytes,
        );
        if accepted {
            self.camera_retry_keys.remove(&key);
        } else if self.camera_retry_eligible(key, generation) {
            self.camera_retry_keys.insert(key);
        }
        accepted
    }

    fn camera_retry_eligible(&self, key: super::ScenePayloadKey, generation: u64) -> bool {
        self.generation_is_current(key.scene_id, generation)
            && !self.terminal_failures.contains(&(key, generation))
            && self.records.get(&key).is_some_and(|record| {
                record.generation == generation
                    && record.state == super::PayloadResidencyState::Unloaded
                    && record.reasons.contains(&ResidencyReason::CameraNear)
            })
    }

    pub(crate) fn update_camera_near(&mut self, admission: CameraAdmission) -> bool {
        let sample = admission.sample;
        if self
            .last_camera
            .is_some_and(|last| !last.changed_meaningfully(sample))
        {
            let Some(generation) = self.active_scene.as_ref().map(|scene| scene.generation) else {
                return false;
            };
            if self.camera_retry_keys.is_empty() {
                return false;
            }
            return self.retry_unloaded_camera_demand(generation);
        }
        let (generation, candidates) = {
            let Some(scene) = self.active_scene.as_ref() else {
                return false;
            };
            let region =
                CameraRegion::around(sample.position, sample.search_radius, sample.preload_margin);
            (
                scene.generation,
                scene
                    .spatial
                    .candidates(region)
                    .filter(|candidate| candidate.admitted_to(&admission))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        let next_keys = candidates
            .iter()
            .map(|candidate| candidate.payload_key)
            .collect::<std::collections::BTreeSet<_>>();
        let old_keys = self.camera_keys.clone();
        for key in old_keys.difference(&next_keys).copied().collect::<Vec<_>>() {
            self.remove_reason(key, ResidencyReason::CameraNear, generation);
        }
        for candidate in candidates {
            self.request_camera_near(
                candidate.payload_key,
                generation,
                candidate.cpu_bytes,
                candidate.gpu_bytes,
            );
        }
        self.camera_keys = next_keys;
        self.last_camera = Some(sample);
        true
    }

    /// Refill only active CameraNear records that previously lost a queue
    /// admission. This avoids a full Scene query while allowing a stationary
    /// camera to make progress as bounded loader slots reopen.
    fn retry_unloaded_camera_demand(&mut self, generation: u64) -> bool {
        let available_slots = super::DEFAULT_LOADER_CAPACITY.saturating_sub(self.loader.len());
        if available_slots == 0 || self.camera_retry_keys.is_empty() {
            return false;
        }
        let retry_keys = self
            .camera_retry_keys
            .iter()
            .copied()
            .take(available_slots)
            .collect::<Vec<_>>();
        let mut progressed = false;
        for key in retry_keys {
            self.camera_retry_keys.remove(&key);
            let Some((cpu_bytes, gpu_bytes)) = self.records.get(&key).and_then(|record| {
                (record.generation == generation
                    && record.state == super::PayloadResidencyState::Unloaded
                    && record.reasons.contains(&ResidencyReason::CameraNear))
                .then_some((record.cpu_bytes, record.gpu_bytes))
            }) else {
                continue;
            };
            if self.request_camera_near(
                key,
                generation,
                cpu_bytes,
                gpu_bytes,
            ) {
                progressed = true;
            }
        }
        progressed
    }
}
