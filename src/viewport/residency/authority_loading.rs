use bevy::asset::Assets;
use bevy::mesh::Mesh;

#[cfg(test)]
use bevy::asset::RenderAssetUsages;
#[cfg(test)]
use bevy::mesh::PrimitiveTopology;

use super::super::loader::LoadJob;
use super::super::repair_phase::RepairPhase;
use super::{PayloadResidencyState, ResidencyAuthority, ScenePayloadKey};

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
                    && record.state == PayloadResidencyState::Loading
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
        if !self.repair_waiting_is_current(job) || !self.loader.enqueue(job.clone()) {
            return false;
        }
        self.repair_phases.insert((job.key, job.generation), phase);
        if let Some(record) = self.records.get_mut(&job.key) {
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
                        && record.state == PayloadResidencyState::Queued
                    {
                        record.state = PayloadResidencyState::Unloaded;
                    }
                }
                continue;
            }
            let valid = self.records.get(&next.key).is_some_and(|record| {
                record.generation == next.generation
                    && record.state == PayloadResidencyState::Queued
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
            record.state = PayloadResidencyState::Loading;
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
        self.complete_cpu_payload(key, generation, cpu_bytes, gpu_bytes, Some(test_mesh()))
    }

    pub(crate) fn complete_cached_cpu(
        &mut self,
        job: LoadJob<ScenePayloadKey>,
        mesh: Mesh,
    ) -> bool {
        let (cpu_bytes, gpu_bytes) = resident_mesh_footprint(&mesh);
        self.complete_cpu_payload(job.key, job.generation, cpu_bytes, gpu_bytes, Some(mesh))
    }

    pub(crate) fn defer_load(&mut self, job: LoadJob<ScenePayloadKey>) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let record_key = job.key;
        let valid = self.records.get(&record_key).is_some_and(|record| {
            record.generation == job.generation && record.state == PayloadResidencyState::Loading
        });
        if !valid {
            return false;
        }
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        if !self.loader.enqueue(job) {
            if let Some(record) = self.records.get_mut(&record_key) {
                record.state = PayloadResidencyState::Unloaded;
            }
            return false;
        }
        if let Some(record) = self.records.get_mut(&record_key) {
            record.state = PayloadResidencyState::Queued;
        }
        true
    }

    pub(crate) fn reject_load(&mut self, job: &LoadJob<ScenePayloadKey>) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let valid = self.records.get(&job.key).is_some_and(|record| {
            record.generation == job.generation && record.state == PayloadResidencyState::Loading
        });
        if !valid {
            return false;
        }
        self.clear_repair_phase(job);
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        let retry_camera = {
            let Some(record) = self.records.get_mut(&job.key) else {
                return false;
            };
            record.state = PayloadResidencyState::Unloaded;
            record.cpu_payload = None;
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
            record.generation == job.generation && record.state == PayloadResidencyState::Loading
        });
        if !valid {
            return false;
        }
        self.clear_repair_phase(job);
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        self.terminal_failures.insert((job.key, job.generation));
        self.camera_retry_keys.remove(&job.key);
        let Some(record) = self.records.get_mut(&job.key) else {
            return false;
        };
        record.state = PayloadResidencyState::Unloaded;
        record.cpu_payload = None;
        true
    }

    pub(crate) fn pump_uploads(
        &mut self,
        assets: &mut Assets<Mesh>,
        render_budget: Option<usize>,
    ) -> Vec<ScenePayloadKey> {
        let mut remaining = self
            .budgets
            .upload_bytes_per_frame
            .min(render_budget.unwrap_or(usize::MAX));
        let mut uploaded = Vec::new();
        for _ in 0..self.ready_for_upload.len() {
            let Some(key) = self.pop_ready_upload() else {
                break;
            };
            let Some((gpu_bytes, cpu_bytes, has_reasons)) = self
                .records
                .get(&key)
                .filter(|record| record.state == PayloadResidencyState::CpuReady)
                .map(|record| {
                    (
                        record.gpu_bytes,
                        record.cpu_bytes,
                        !record.reasons.is_empty(),
                    )
                })
            else {
                continue;
            };
            let required = usize::try_from(gpu_bytes).unwrap_or(usize::MAX);
            if required > remaining && !uploaded.is_empty() {
                self.requeue_ready_upload_front(key);
                break;
            }
            if !self.ensure_capacity(0, gpu_bytes) {
                if gpu_bytes > self.budgets.gpu_bytes {
                    self.cpu_used = self.cpu_used.saturating_sub(cpu_bytes);
                    if let Some(record) = self.records.get_mut(&key) {
                        record.state = PayloadResidencyState::Unloaded;
                        record.cpu_payload = None;
                    }
                    continue;
                }
                if self.ready_upload_membership.insert(key) {
                    self.ready_for_upload.push_back(key);
                }
                continue;
            }
            remaining = if required > remaining {
                0
            } else {
                remaining.saturating_sub(required)
            };
            let Some(record) = self.records.get_mut(&key) else {
                continue;
            };
            let Some(mesh) = record.cpu_payload.take() else {
                continue;
            };
            // Inserting the decoded cache payload into Bevy's Assets<Mesh> is
            // the owner boundary. Bevy's RenderAssetBytesPerFrame then
            // throttles the actual render-world transfer.
            let handle = assets.add(mesh);
            record.render_handle = Some(handle);
            self.gpu_used += gpu_bytes;
            record.state = PayloadResidencyState::GpuResident;
            uploaded.push(key);
            if !has_reasons {
                record.state = PayloadResidencyState::Warm;
                self.warm.insert(key, cpu_bytes, gpu_bytes, false);
            }
        }
        self.evict_to_budget();
        uploaded
    }

    fn complete_cpu_payload(
        &mut self,
        key: ScenePayloadKey,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
        cpu_payload: Option<Mesh>,
    ) -> bool {
        if !self.generation_is_current(key.scene_id, generation) {
            return false;
        }
        let Some(reserved_bytes) = self
            .records
            .get(&key)
            .filter(|record| {
                record.generation == generation && record.state == PayloadResidencyState::Loading
            })
            .map(|record| record.cpu_bytes)
        else {
            return false;
        };
        self.repair_phases.remove(&(key, generation));
        self.cpu_reserved = self.cpu_reserved.saturating_sub(reserved_bytes);
        if !self.ensure_capacity(cpu_bytes, 0) || gpu_bytes > self.budgets.gpu_bytes {
            if let Some(record) = self.records.get_mut(&key) {
                record.state = PayloadResidencyState::Unloaded;
                record.cpu_payload = None;
            }
            return false;
        }
        let has_reasons = {
            let Some(record) = self.records.get_mut(&key) else {
                return false;
            };
            record.cpu_bytes = cpu_bytes;
            record.gpu_bytes = gpu_bytes;
            record.cpu_payload = cpu_payload;
            let has_reasons = !record.reasons.is_empty();
            record.state = if has_reasons {
                PayloadResidencyState::CpuReady
            } else {
                PayloadResidencyState::Warm
            };
            has_reasons
        };
        self.cpu_used += cpu_bytes;
        if has_reasons {
            self.enqueue_ready_upload(key);
        } else {
            self.warm.insert(key, cpu_bytes, 0, false);
        }
        self.evict_to_budget();
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
