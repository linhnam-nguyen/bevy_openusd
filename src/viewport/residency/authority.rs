//! Viewport-owned residency authority and its generation-safe state machine.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use bevy::asset::{AssetId, Handle};
use bevy::mesh::Mesh;
use bevy::prelude::Resource;
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache_contract::SceneCacheEntry;

use super::camera::CameraSample;
use super::loader::{BoundedLoader, LoadJob};
use super::lru::WarmLru;
use super::repair_phase::RepairPhase;
use super::spatial::CameraCandidateIndex;
use super::catalog::PayloadLoadMask;

#[path = "authority_camera.rs"]
mod authority_camera;
#[path = "authority_loading.rs"]
mod authority_loading;
#[path = "authority_runtime.rs"]
mod authority_runtime;
#[path = "authority_capacity.rs"]
mod authority_capacity;
#[path = "authority_completion.rs"]
mod authority_completion;
#[path = "authority_payloads.rs"]
mod authority_payloads;

pub(crate) const DEFAULT_LOADER_CAPACITY: usize = 256;
pub(crate) const DEFAULT_UPLOAD_BYTES_PER_FRAME: usize = 4 * 1024 * 1024;
pub(crate) const RESIDENT_CPU_METADATA_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ResidencyReason {
    CameraNear,
    Selected,
    AnimationRequired,
    ActiveViewpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PayloadResidencyState {
    Unloaded,
    Queued,
    Loading,
    RepairWaiting,
    CpuReady,
    GpuResident,
    Warm,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ScenePayloadKey {
    pub(crate) scene_id: SceneId,
    pub(crate) blob_hash: HashDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResidencyBudgets {
    pub(crate) cpu_bytes: u64,
    pub(crate) gpu_bytes: u64,
    pub(crate) upload_bytes_per_frame: usize,
}

impl Default for ResidencyBudgets {
    fn default() -> Self {
        Self {
            cpu_bytes: 512 * 1024 * 1024,
            gpu_bytes: 512 * 1024 * 1024,
            upload_bytes_per_frame: DEFAULT_UPLOAD_BYTES_PER_FRAME,
        }
    }
}

#[derive(Clone, Debug)]
struct PayloadRecord {
    generation: u64,
    state: PayloadResidencyState,
    reasons: BTreeSet<ResidencyReason>,
    requested_mask: PayloadLoadMask,
    satisfied_mask: PayloadLoadMask,
    queued_mask: PayloadLoadMask,
    in_flight_mask: PayloadLoadMask,
    cpu_bytes: u64,
    gpu_bytes: u64,
    cpu_payload: Option<Mesh>,
    render_handle: Option<Handle<Mesh>>,
}

#[derive(Debug)]
struct ActiveScene {
    scene_id: SceneId,
    generation: u64,
    spatial: CameraCandidateIndex,
}

#[derive(Resource, Debug)]
pub(crate) struct ResidencyAuthority {
    budgets: ResidencyBudgets,
    scene_generations: HashMap<SceneId, u64>,
    records: HashMap<ScenePayloadKey, PayloadRecord>,
    loader: BoundedLoader<ScenePayloadKey>,
    repair_phases: HashMap<(ScenePayloadKey, u64), RepairPhase>,
    terminal_failures: HashSet<(ScenePayloadKey, u64)>,
    ready_for_upload: VecDeque<ScenePayloadKey>,
    ready_upload_membership: HashSet<ScenePayloadKey>,
    warm: WarmLru<ScenePayloadKey>,
    pending_loads: BTreeSet<ScenePayloadKey>,
    cpu_used: u64,
    cpu_reserved: u64,
    gpu_used: u64,
    active_scene: Option<ActiveScene>,
    camera_keys: BTreeSet<ScenePayloadKey>,
    camera_retry_keys: BTreeSet<ScenePayloadKey>,
    last_camera: Option<CameraSample>,
    released_render_assets: Vec<AssetId<Mesh>>,
}

impl Default for ResidencyAuthority {
    fn default() -> Self {
        Self::with_budgets(ResidencyBudgets::default())
    }
}

impl ResidencyAuthority {
    pub(crate) fn with_budgets(budgets: ResidencyBudgets) -> Self {
        Self {
            budgets,
            scene_generations: HashMap::new(),
            records: HashMap::new(),
            loader: BoundedLoader::new(DEFAULT_LOADER_CAPACITY),
            repair_phases: HashMap::new(),
            terminal_failures: HashSet::new(),
            ready_for_upload: VecDeque::new(),
            ready_upload_membership: HashSet::new(),
            warm: WarmLru::default(),
            pending_loads: BTreeSet::new(),
            cpu_used: 0,
            cpu_reserved: 0,
            gpu_used: 0,
            active_scene: None,
            camera_keys: BTreeSet::new(),
            camera_retry_keys: BTreeSet::new(),
            last_camera: None,
            released_render_assets: Vec::new(),
        }
    }
    pub(crate) fn budgets(&self) -> ResidencyBudgets {
        self.budgets
    }

    pub(crate) fn load_mask(&self, key: &ScenePayloadKey) -> PayloadLoadMask {
        self.records
            .get(key)
            .map(|record| record.requested_mask)
            .unwrap_or(PayloadLoadMask::GEOMETRY)
    }
    pub(crate) fn set_budgets(&mut self, budgets: ResidencyBudgets) {
        self.budgets = budgets;
        self.evict_to_budget();
    }
    pub(crate) fn request_reason(
        &mut self,
        key: ScenePayloadKey,
        reason: ResidencyReason,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
    ) -> bool {
        if !self.generation_is_current(key.scene_id, generation) {
            return false;
        }
        let suppressed = self.terminal_failures.contains(&(key, generation));
        let was_warm = self
            .records
            .get(&key)
            .is_some_and(|record| record.state == PayloadResidencyState::Warm);
        if was_warm {
            self.warm.remove(&key);
        }
        let record = self.records.entry(key).or_insert_with(|| PayloadRecord {
            generation,
            state: PayloadResidencyState::Unloaded,
            reasons: BTreeSet::new(),
            requested_mask: PayloadLoadMask::default(),
            satisfied_mask: PayloadLoadMask::default(),
            queued_mask: PayloadLoadMask::default(),
            in_flight_mask: PayloadLoadMask::default(),
            cpu_bytes,
            gpu_bytes,
            cpu_payload: None,
            render_handle: None,
        });
        if record.generation != generation {
            return false;
        }
        if matches!(
            record.state,
            PayloadResidencyState::Unloaded
                | PayloadResidencyState::Queued
                | PayloadResidencyState::Loading
                | PayloadResidencyState::RepairWaiting
        ) {
            record.cpu_bytes = cpu_bytes;
            record.gpu_bytes = gpu_bytes;
        }
        record.reasons.insert(reason);
        record.requested_mask = record
            .requested_mask
            .union(PayloadLoadMask::for_reason(reason));
        if suppressed {
            return false;
        }
        if was_warm {
            record.state = if record.render_handle.is_some() {
                    PayloadResidencyState::GpuResident
                } else {
                    PayloadResidencyState::CpuReady
                };
            if record.state == PayloadResidencyState::CpuReady {
                self.enqueue_ready_upload(key);
            }
        }
        self.enqueue_missing_load(key)
    }
    pub(crate) fn remove_reason(
        &mut self,
        key: ScenePayloadKey,
        reason: ResidencyReason,
        generation: u64,
    ) -> bool {
        let Some(existing) = self.records.get(&key) else {
            return false;
        };
        if existing.generation != generation {
            return false;
        }
        if reason == ResidencyReason::CameraNear {
            self.camera_retry_keys.remove(&key);
        }
        let was_cpu_ready = existing.state == PayloadResidencyState::CpuReady;
        let has_reasons = {
            let record = self.records.get_mut(&key).expect("record still exists");
            record.reasons.remove(&reason);
            record.requested_mask = record
                .reasons
                .iter()
                .fold(PayloadLoadMask::default(), |mask, reason| {
                    mask.union(PayloadLoadMask::for_reason(*reason))
                });
            !record.reasons.is_empty()
        };
        if has_reasons {
            return true;
        }
        self.repair_phases.remove(&(key, generation));
        self.pending_loads.remove(&key);
        let queued_mask = self
            .records
            .get(&key)
            .map_or(PayloadLoadMask::default(), |record| record.queued_mask);
        if !queued_mask.is_empty() {
            self.loader.cancel(&key);
            if let Some(record) = self.records.get_mut(&key) {
                record.queued_mask = PayloadLoadMask::default();
            }
        }
        if was_cpu_ready {
            self.remove_ready_upload(&key);
        }
        let record = self.records.get_mut(&key).expect("record still exists");
        match record.state {
            PayloadResidencyState::GpuResident => {
                record.state = PayloadResidencyState::Warm;
                self.warm
                    .insert(key, record.cpu_bytes, record.gpu_bytes, false);
            }
            PayloadResidencyState::CpuReady => {
                record.state = PayloadResidencyState::Warm;
                self.warm.insert(key, record.cpu_bytes, 0, false);
            }
            PayloadResidencyState::Queued => {
                record.state = PayloadResidencyState::Unloaded;
            }
            PayloadResidencyState::RepairWaiting => {
                record.state = PayloadResidencyState::Unloaded;
            }
            _ => {}
        }
        self.evict_to_budget();
        true
    }
    pub(crate) fn state(&self, key: &ScenePayloadKey) -> Option<PayloadResidencyState> {
        self.records.get(key).map(|record| record.state)
    }
    pub(crate) fn reasons(&self, key: &ScenePayloadKey) -> Option<&BTreeSet<ResidencyReason>> {
        self.records.get(key).map(|record| &record.reasons)
    }

    pub(crate) fn accounted_bytes(&self) -> (u64, u64) {
        (self.cpu_used, self.gpu_used)
    }
    pub(crate) fn queue_len(&self) -> usize {
        self.loader.len()
    }
    pub(crate) fn warm_len(&self) -> usize {
        self.warm.len()
    }
    pub(crate) fn render_asset_id(&self, key: &ScenePayloadKey) -> Option<AssetId<Mesh>> {
        self.records
            .get(key)
            .and_then(|record| record.render_handle.as_ref().map(|handle| handle.id()))
    }
    pub(crate) fn render_handle(&self, key: &ScenePayloadKey) -> Option<Handle<Mesh>> {
        self.records
            .get(key)
            .and_then(|record| record.render_handle.clone())
    }

    pub(crate) fn take_released_render_assets(&mut self) -> Vec<AssetId<Mesh>> {
        std::mem::take(&mut self.released_render_assets)
    }
    pub(crate) fn payload_key(entry: &SceneCacheEntry) -> Option<ScenePayloadKey> {
        entry.content_hash.map(|blob_hash| ScenePayloadKey {
            scene_id: entry.address.scene_id,
            blob_hash,
        })
    }

    pub(crate) fn conservative_resident_footprint(encoded_bytes: u64) -> (u64, u64) {
        let gpu_bytes = encoded_bytes.saturating_mul(4).max(1);
        (
            gpu_bytes.saturating_add(RESIDENT_CPU_METADATA_BYTES),
            gpu_bytes,
        )
    }

    fn generation_is_current(&self, scene_id: SceneId, generation: u64) -> bool {
        self.scene_generations
            .get(&scene_id)
            .is_some_and(|current| *current == generation)
    }

}

#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "authority_payload_satisfaction_tests.rs"]
mod payload_satisfaction_tests;
