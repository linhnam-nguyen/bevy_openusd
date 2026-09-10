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
use super::spatial::{CameraCandidateIndex, SceneSpatialPayload};

#[path = "authority_camera.rs"]
mod authority_camera;
#[path = "authority_loading.rs"]
mod authority_loading;
#[path = "authority_runtime.rs"]
mod authority_runtime;

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
    terminal_failures: HashSet<(ScenePayloadKey, u64)>,
    ready_for_upload: VecDeque<ScenePayloadKey>,
    ready_upload_membership: HashSet<ScenePayloadKey>,
    warm: WarmLru<ScenePayloadKey>,
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
            terminal_failures: HashSet::new(),
            ready_for_upload: VecDeque::new(),
            ready_upload_membership: HashSet::new(),
            warm: WarmLru::default(),
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
        ) {
            record.cpu_bytes = cpu_bytes;
            record.gpu_bytes = gpu_bytes;
        }
        record.reasons.insert(reason);
        if suppressed {
            return false;
        }
        if was_warm {
            let needs_upload = {
                record.state = if record.render_handle.is_some() {
                    PayloadResidencyState::GpuResident
                } else {
                    PayloadResidencyState::CpuReady
                };
                record.state == PayloadResidencyState::CpuReady
            };
            if needs_upload {
                self.enqueue_ready_upload(key);
            }
            return true;
        }
        if record.state == PayloadResidencyState::Unloaded {
            let accepted = self.loader.enqueue(LoadJob {
                key,
                generation,
                cpu_bytes,
                gpu_bytes,
            });
            if accepted {
                record.state = PayloadResidencyState::Queued;
            }
            return accepted;
        }
        true
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
            !record.reasons.is_empty()
        };
        if has_reasons {
            return true;
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
                self.loader.cancel(&key);
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

    fn enqueue_ready_upload(&mut self, key: ScenePayloadKey) {
        if self.ready_upload_membership.insert(key) {
            self.ready_for_upload.push_back(key);
        }
    }

    fn remove_ready_upload(&mut self, key: &ScenePayloadKey) {
        if !self.ready_upload_membership.remove(key) {
            return;
        }
        let position = self.ready_for_upload.iter().position(|queued| queued == key);
        debug_assert!(position.is_some());
        if let Some(position) = position {
            let _ = self.ready_for_upload.remove(position);
        }
    }

    fn pop_ready_upload(&mut self) -> Option<ScenePayloadKey> {
        let key = self.ready_for_upload.pop_front()?;
        let removed = self.ready_upload_membership.remove(&key);
        debug_assert!(removed);
        Some(key)
    }

    fn requeue_ready_upload_front(&mut self, key: ScenePayloadKey) {
        if self.ready_upload_membership.insert(key) {
            self.ready_for_upload.push_front(key);
        }
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

    fn evict_to_budget(&mut self) {
        while self.cpu_used > self.budgets.cpu_bytes || self.gpu_used > self.budgets.gpu_bytes {
            if !self.evict_oldest_warm() {
                break;
            }
        }
    }

    fn evict_oldest_warm(&mut self) -> bool {
        let Some((key, cpu_bytes, gpu_bytes)) = self.warm.pop_oldest_unpinned() else {
            return false;
        };
        self.cpu_used = self.cpu_used.saturating_sub(cpu_bytes);
        self.gpu_used = self.gpu_used.saturating_sub(gpu_bytes);
        if let Some(record) = self.records.get_mut(&key) {
            record.state = PayloadResidencyState::Unloaded;
            record.cpu_payload = None;
            if let Some(handle) = record.render_handle.take() {
                self.released_render_assets.push(handle.id());
            }
        }
        true
    }

    fn can_fit_cpu(&self, bytes: u64) -> bool {
        self.cpu_used <= self.budgets.cpu_bytes
            && self.cpu_reserved <= self.budgets.cpu_bytes - self.cpu_used
            && bytes <= self.budgets.cpu_bytes - self.cpu_used - self.cpu_reserved
    }

    fn can_fit_gpu(&self, bytes: u64) -> bool {
        self.gpu_used <= self.budgets.gpu_bytes && bytes <= self.budgets.gpu_bytes - self.gpu_used
    }

    fn ensure_capacity(&mut self, cpu_bytes: u64, gpu_bytes: u64) -> bool {
        if cpu_bytes > self.budgets.cpu_bytes || gpu_bytes > self.budgets.gpu_bytes {
            return false;
        }
        while !self.can_fit_cpu(cpu_bytes) || !self.can_fit_gpu(gpu_bytes) {
            if !self.evict_oldest_warm() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
