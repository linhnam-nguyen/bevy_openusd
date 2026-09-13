use bevy::mesh::Mesh;

#[cfg(test)]
use bevy::asset::RenderAssetUsages;
#[cfg(test)]
use bevy::mesh::PrimitiveTopology;

use super::super::loader::LoadJob;
use super::super::repair_phase::RepairPhase;
use super::{PayloadLoadMask, PayloadResidencyState, ResidencyAuthority, ScenePayloadKey};

impl ResidencyAuthority {
    pub(crate) fn take_repair_phase(&mut self, job: &LoadJob<ScenePayloadKey>) -> RepairPhase {
        self.repair_phases
            .remove(&(job.key, job.generation))
            .unwrap_or(RepairPhase::NeedScenePayload)
    }

    pub(crate) fn clear_repair_phase(&mut self, job: &LoadJob<ScenePayloadKey>) {
        self.repair_phases.remove(&(job.key, job.generation));
    }

    pub(crate) fn load_is_current(&self, job: &LoadJob<ScenePayloadKey>) -> bool {
            self.generation_is_current(job.key.scene_id, job.generation)
            && self.records.get(&job.key).is_some_and(|record| {
                record.generation == job.generation
                    && !record.in_flight_mask.is_empty()
            })
    }

    pub(crate) fn repair_waiting_is_current(&self, job: &LoadJob<ScenePayloadKey>) -> bool {
        self.generation_is_current(job.key.scene_id, job.generation)
            && self.records.get(&job.key).is_some_and(|record| {
                record.generation == job.generation
                    && record.state == PayloadResidencyState::RepairWaiting
            })
    }

    pub(crate) fn wait_for_repair(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.load_is_current(job) {
            return false;
        }
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        if let Some(record) = self.records.get_mut(&job.key) {
            record.in_flight_mask = PayloadLoadMask::default();
            record.state = PayloadResidencyState::RepairWaiting;
            return true;
        }
        false
    }

    pub(crate) fn requeue_repair_waiting(
        &mut self,
        job: &LoadJob<ScenePayloadKey>,
        phase: RepairPhase,
    ) -> bool {
        if !self.repair_waiting_is_current(job) {
            return false;
        }
        let queued_mask = self
            .records
            .get(&job.key)
            .map(|record| {
                self.load_mask(&job.key)
                    .missing_from(record.satisfied_mask)
            })
            .filter(|mask| !mask.is_empty())
            .unwrap_or(PayloadLoadMask::GEOMETRY);
        if !self.loader.enqueue(job.clone()) {
            return false;
        }
        self.repair_phases.insert((job.key, job.generation), phase);
        if let Some(record) = self.records.get_mut(&job.key) {
            record.queued_mask = queued_mask;
            record.state = PayloadResidencyState::Queued;
            return true;
        }
        false
    }

    pub(crate) fn reject_repair(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.repair_waiting_is_current(job) {
            return false;
        }
        self.clear_repair_phase(job);
        let retry_camera = self
            .records
            .get(&job.key)
            .is_some_and(|record| record.reasons.contains(&super::ResidencyReason::CameraNear));
        if let Some(record) = self.records.get_mut(&job.key) {
            record.in_flight_mask = PayloadLoadMask::default();
            record.state = PayloadResidencyState::Unloaded;
            record.cpu_payload = None;
        }
        if retry_camera {
            self.camera_retry_keys.insert(job.key);
        }
        true
    }

    pub(crate) fn suppress_repair(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.repair_waiting_is_current(job) {
            return false;
        }
        self.clear_repair_phase(job);
        self.terminal_failures.insert((job.key, job.generation));
        self.camera_retry_keys.remove(&job.key);
        if let Some(record) = self.records.get_mut(&job.key) {
            record.in_flight_mask = PayloadLoadMask::default();
            record.state = PayloadResidencyState::Unloaded;
            record.cpu_payload = None;
            return true;
        }
        false
    }

    pub(crate) fn begin_next_load(&mut self) -> Option<LoadJob<ScenePayloadKey>> {
        let queued = self.loader.len();
        let mut deferred = Vec::new();
        for _ in 0..queued {
            let Some(next) = self.loader.pop() else {
                break;
            };
            // An individually oversized payload is deterministically skipped;
            // it can never fit without violating the hard budget contract.
            if next.cpu_bytes > self.budgets.cpu_bytes || next.gpu_bytes > self.budgets.gpu_bytes {
            if let Some(record) = self.records.get_mut(&next.key) {
                if record.generation == next.generation
                    && !record.queued_mask.is_empty()
                {
                    record.queued_mask = PayloadLoadMask::default();
                    if !record.satisfied_mask.geometry {
                        record.state = PayloadResidencyState::Unloaded;
                    }
                }
            }
                continue;
            }
            let valid = self.records.get(&next.key).is_some_and(|record| {
                record.generation == next.generation
                    && !record.queued_mask.is_empty()
                    && record.in_flight_mask.is_empty()
            });
            if !valid {
                continue;
            }
            if !self.ensure_capacity(next.cpu_bytes, 0) {
                deferred.push(next);
                continue;
            }
            self.loader.requeue_front_all(&mut deferred);
            let Some(record) = self.records.get_mut(&next.key) else {
                return Some(next);
            };
            let queued_mask = record.queued_mask;
            record.queued_mask = PayloadLoadMask::default();
            record.in_flight_mask = queued_mask;
            if !record.satisfied_mask.geometry {
                record.state = PayloadResidencyState::Loading;
            }
            self.cpu_reserved = self.cpu_reserved.saturating_add(next.cpu_bytes);
            return Some(next);
        }
        self.loader.requeue_front_all(&mut deferred);
        None
    }

    #[cfg(test)]
    pub(crate) fn complete_cpu(
        &mut self,
        key: ScenePayloadKey,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
    ) -> bool {
        let mask = self.in_flight_mask(&key);
        self.complete_cpu_payload(
            key,
            generation,
            cpu_bytes,
            gpu_bytes,
            if mask.is_empty() {
                PayloadLoadMask::GEOMETRY
            } else {
                mask
            },
            Some(test_mesh()),
        )
    }

    pub(crate) fn defer_load(&mut self, job: LoadJob<ScenePayloadKey>) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let record_key = job.key;
        let valid = self.records.get(&record_key).is_some_and(|record| {
            record.generation == job.generation && !record.in_flight_mask.is_empty()
        });
        if !valid {
            return false;
        }
        let (in_flight_mask, previous_state) = self
            .records
            .get(&record_key)
            .map(|record| (record.in_flight_mask, record.state))
            .unwrap_or((PayloadLoadMask::default(), PayloadResidencyState::Unloaded));
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        if !self.loader.enqueue(job) {
            if let Some(record) = self.records.get_mut(&record_key) {
                record.in_flight_mask = PayloadLoadMask::default();
                record.queued_mask = PayloadLoadMask::default();
                if previous_state == PayloadResidencyState::Loading {
                    record.state = PayloadResidencyState::Unloaded;
                }
            }
            return false;
        }
        if let Some(record) = self.records.get_mut(&record_key) {
            record.in_flight_mask = PayloadLoadMask::default();
            record.queued_mask = in_flight_mask;
            if previous_state == PayloadResidencyState::Loading {
                record.state = PayloadResidencyState::Queued;
            }
        }
        true
    }

    pub(crate) fn reject_load(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let valid = self.records.get(&job.key).is_some_and(|record| {
            record.generation == job.generation && !record.in_flight_mask.is_empty()
        });
        if !valid {
            return false;
        }
        self.clear_repair_phase(job);
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        let preserve_resident_geometry = self
            .records
            .get(&job.key)
            .is_some_and(|record| record.satisfied_mask.geometry);
        let retry_camera = {
            let Some(record) = self.records.get_mut(&job.key) else {
                return false;
            };
            record.in_flight_mask = PayloadLoadMask::default();
            if !preserve_resident_geometry {
                record.state = PayloadResidencyState::Unloaded;
                record.cpu_payload = None;
            }
            record.reasons.contains(&super::ResidencyReason::CameraNear)
        };
        if retry_camera {
            self.camera_retry_keys.insert(job.key);
        }
        true
    }

    pub(crate) fn suppress_failed_load(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let valid = self.records.get(&job.key).is_some_and(|record| {
            record.generation == job.generation && !record.in_flight_mask.is_empty()
        });
        if !valid {
            return false;
        }
        self.clear_repair_phase(job);
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        self.terminal_failures.insert((job.key, job.generation));
        self.camera_retry_keys.remove(&job.key);
        let preserve_resident_geometry = self
            .records
            .get(&job.key)
            .is_some_and(|record| record.satisfied_mask.geometry);
        let Some(record) = self.records.get_mut(&job.key) else {
            return false;
        };
        record.in_flight_mask = PayloadLoadMask::default();
        if !preserve_resident_geometry {
            record.state = PayloadResidencyState::Unloaded;
            record.cpu_payload = None;
        }
        true
    }

}

pub(super) fn resident_mesh_footprint(mesh: &Mesh) -> (u64, u64) {
    let vertex_bytes = mesh.get_vertex_buffer_size() as u64;
    let index_bytes = mesh
        .get_index_buffer_bytes()
        .map_or(0, |bytes| bytes.len() as u64);
    let gpu_bytes = vertex_bytes.saturating_add(index_bytes);
    (
        gpu_bytes.saturating_add(super::RESIDENT_CPU_METADATA_BYTES),
        gpu_bytes,
    )
}

#[cfg(test)]
fn test_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
}

#[cfg(test)]
#[path = "authority_loading_tests.rs"]
mod tests;
