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
mod tests {
    use bevy::asset::Assets;
    use bevy::mesh::Mesh;
    use usd_model::HashDigest;
    use usd_project::SceneId;

    use super::super::{
        PayloadResidencyState, ResidencyAuthority, ResidencyBudgets, ResidencyReason,
        ScenePayloadKey,
    };

    fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
        ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
        }
    }

    #[test]
    fn upload_requeue_restores_physical_membership_after_budget_consumed() {
        let scene = SceneId::new_v4();
        let first = key(scene, 1);
        let second = key(scene, 2);
        let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
            cpu_bytes: 32,
            gpu_bytes: 16,
            upload_bytes_per_frame: 4,
        });
        let mut assets = Assets::<Mesh>::default();
        authority.install_scene(scene, 91, Vec::new());
        assert!(authority.request_reason(first, ResidencyReason::CameraNear, 91, 4, 4,));
        assert!(authority.request_reason(second, ResidencyReason::CameraNear, 91, 1, 1,));
        let first_job = authority.begin_next_load().expect("first ready job");
        let second_job = authority.begin_next_load().expect("second ready job");
        assert!(authority.complete_cpu(first_job.key, 91, 4, 4));
        assert!(authority.complete_cpu(second_job.key, 91, 1, 1));
        assert_eq!(authority.ready_for_upload.len(), 2);
        assert_eq!(authority.ready_upload_membership.len(), 2);

        assert_eq!(authority.pump_uploads(&mut assets, Some(4)), vec![first]);
        assert_eq!(
            authority.state(&second),
            Some(PayloadResidencyState::CpuReady)
        );
        assert_eq!(authority.ready_for_upload.front(), Some(&second));
        assert_eq!(authority.ready_for_upload.len(), 1);
        assert_eq!(authority.ready_upload_membership.len(), 1);

        assert_eq!(authority.pump_uploads(&mut assets, Some(4)), vec![second]);
        assert!(authority.ready_for_upload.is_empty());
        assert!(authority.ready_upload_membership.is_empty());
    }

    #[test]
    fn capacity_deferred_ready_payload_rotates_to_smaller_progress() {
        let scene = SceneId::new_v4();
        let occupied = key(scene, 1);
        let larger = key(scene, 2);
        let smaller = key(scene, 3);
        let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
            cpu_bytes: 32,
            gpu_bytes: 10,
            upload_bytes_per_frame: 16,
        });
        let mut assets = Assets::<Mesh>::default();
        authority.install_scene(scene, 92, Vec::new());

        assert!(authority.request_reason(occupied, ResidencyReason::CameraNear, 92, 8, 8,));
        let occupied_job = authority.begin_next_load().expect("occupied job");
        assert!(authority.complete_cpu(occupied_job.key, 92, 8, 8));
        assert_eq!(
            authority.pump_uploads(&mut assets, Some(16)),
            vec![occupied]
        );
        assert_eq!(authority.accounted_bytes(), (8, 8));

        assert!(authority.request_reason(larger, ResidencyReason::CameraNear, 92, 4, 4));
        assert!(authority.request_reason(smaller, ResidencyReason::CameraNear, 92, 1, 1));
        let larger_job = authority.begin_next_load().expect("larger ready job");
        let smaller_job = authority.begin_next_load().expect("smaller ready job");
        assert!(authority.complete_cpu(larger_job.key, 92, 4, 4));
        assert!(authority.complete_cpu(smaller_job.key, 92, 1, 1));

        assert_eq!(authority.pump_uploads(&mut assets, Some(16)), vec![smaller]);
        assert_eq!(
            authority.state(&larger),
            Some(PayloadResidencyState::CpuReady)
        );
        assert_eq!(authority.ready_for_upload.front(), Some(&larger));
        assert_eq!(authority.ready_for_upload.len(), 1);
        assert_eq!(authority.ready_upload_membership.len(), 1);
        assert!(authority.accounted_bytes().1 <= authority.budgets().gpu_bytes);
    }

    #[test]
    fn cpu_capacity_skips_blocked_head_and_preserves_queue_order() {
        let scene = SceneId::new_v4();
        let active = key(scene, 4);
        let blocked = key(scene, 5);
        let later = key(scene, 6);
        let mut authority = ResidencyAuthority::with_budgets(ResidencyBudgets {
            cpu_bytes: 8,
            gpu_bytes: 8,
            upload_bytes_per_frame: 8,
        });
        authority.install_scene(scene, 93, Vec::new());

        assert!(authority.request_reason(active, ResidencyReason::Selected, 93, 6, 1));
        assert!(authority.request_reason(blocked, ResidencyReason::Selected, 93, 3, 1));
        assert!(authority.request_reason(later, ResidencyReason::Selected, 93, 1, 1));
        let active_job = authority.begin_next_load().expect("active payload");
        assert!(authority.complete_cpu(active_job.key, 93, 6, 1));

        let later_job = authority
            .begin_next_load()
            .expect("later job fits remaining CPU capacity");
        assert_eq!(later_job.key, later);
        assert_eq!(
            authority.state(&active),
            Some(PayloadResidencyState::CpuReady)
        );
        assert_eq!(
            authority.state(&blocked),
            Some(PayloadResidencyState::Queued)
        );
        assert_eq!(authority.loader.peek().map(|job| job.key), Some(blocked));
        assert_eq!(authority.queue_len(), 1);
        assert_eq!(authority.cpu_used, 6);
        assert_eq!(authority.cpu_reserved, 1);
        assert!(authority.cpu_used + authority.cpu_reserved <= authority.budgets.cpu_bytes);
    }
}
