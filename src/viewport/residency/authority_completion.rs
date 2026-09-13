use bevy::asset::Assets;
use bevy::mesh::Mesh;

use super::authority_loading::resident_mesh_footprint;
use super::super::loader::LoadJob;
use super::{PayloadLoadMask, PayloadResidencyState, ResidencyAuthority, ScenePayloadKey};

impl ResidencyAuthority {
    pub(crate) fn complete_cached_cpu(
        &mut self,
        job: LoadJob<ScenePayloadKey>,
        mesh: Mesh,
    ) -> bool {
        let mask = self.in_flight_mask(&job.key);
        self.complete_cached_cpu_with_mask(job, mesh, mask)
    }

    pub(crate) fn complete_cached_cpu_with_mask(
        &mut self,
        job: LoadJob<ScenePayloadKey>,
        mesh: Mesh,
        mask: PayloadLoadMask,
    ) -> bool {
        let (cpu_bytes, gpu_bytes) = resident_mesh_footprint(&mesh);
        self.complete_cpu_payload(
            job.key,
            job.generation,
            cpu_bytes,
            gpu_bytes,
            mask,
            Some(mesh),
        )
    }

    pub(crate) fn complete_cached_payloads(
        &mut self,
        job: &LoadJob<ScenePayloadKey>,
        mask: PayloadLoadMask,
    ) -> bool {
        if !self.generation_is_current(job.key.scene_id, job.generation) {
            return false;
        }
        let Some(in_flight_mask) = self
            .records
            .get(&job.key)
            .map(|record| record.in_flight_mask)
        else {
            return false;
        };
        if mask.geometry || mask.is_empty() || !in_flight_mask.contains(mask) {
            return false;
        }
        self.cpu_reserved = self.cpu_reserved.saturating_sub(job.cpu_bytes);
        let Some((became_warm, cpu_bytes, gpu_bytes)) = self.records.get_mut(&job.key).map(|record| {
            record.in_flight_mask = PayloadLoadMask::default();
            record.satisfied_mask = record.satisfied_mask.union(mask);
            let became_warm = record.reasons.is_empty()
                && record.state == PayloadResidencyState::GpuResident;
            if became_warm {
                record.state = PayloadResidencyState::Warm;
            }
            (became_warm, record.cpu_bytes, record.gpu_bytes)
        }) else {
            return false;
        };
        if became_warm {
            self.warm.insert(job.key, cpu_bytes, gpu_bytes, false);
        }
        self.enqueue_missing_load(job.key);
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

    pub(super) fn complete_cpu_payload(
        &mut self,
        key: ScenePayloadKey,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
        loaded_mask: PayloadLoadMask,
        cpu_payload: Option<Mesh>,
    ) -> bool {
        if !self.generation_is_current(key.scene_id, generation) {
            return false;
        }
        let Some((reserved_bytes, in_flight_mask)) = self
            .records
            .get(&key)
            .filter(|record| {
                record.generation == generation
                    && !record.in_flight_mask.is_empty()
                    && record.in_flight_mask.contains(loaded_mask)
            })
            .map(|record| (record.cpu_bytes, record.in_flight_mask))
        else {
            return false;
        };
        if !loaded_mask.geometry {
            return false;
        }
        self.repair_phases.remove(&(key, generation));
        self.cpu_reserved = self.cpu_reserved.saturating_sub(reserved_bytes);
        if !self.ensure_capacity(cpu_bytes, 0) || gpu_bytes > self.budgets.gpu_bytes {
            if let Some(record) = self.records.get_mut(&key) {
                record.in_flight_mask = PayloadLoadMask::default();
                if !record.satisfied_mask.geometry {
                    record.state = PayloadResidencyState::Unloaded;
                    record.cpu_payload = None;
                }
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
            record.in_flight_mask = PayloadLoadMask::default();
            record.satisfied_mask = record.satisfied_mask.union(in_flight_mask);
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
        self.enqueue_missing_load(key);
        self.evict_to_budget();
        true
    }
}
