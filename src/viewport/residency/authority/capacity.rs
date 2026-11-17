use super::{ResidencyAuthority, ScenePayloadKey};

impl ResidencyAuthority {
    pub(super) fn enqueue_ready_upload(&mut self, key: ScenePayloadKey) {
        if self.ready_upload_membership.insert(key) {
            self.ready_for_upload.push_back(key);
        }
    }

    pub(super) fn remove_ready_upload(&mut self, key: &ScenePayloadKey) {
        if !self.ready_upload_membership.remove(key) {
            return;
        }
        let position = self
            .ready_for_upload
            .iter()
            .position(|queued| queued == key);
        debug_assert!(position.is_some());
        if let Some(position) = position {
            let _ = self.ready_for_upload.remove(position);
        }
    }

    pub(super) fn pop_ready_upload(&mut self) -> Option<ScenePayloadKey> {
        let key = self.ready_for_upload.pop_front()?;
        let removed = self.ready_upload_membership.remove(&key);
        debug_assert!(removed);
        Some(key)
    }

    pub(super) fn requeue_ready_upload_front(&mut self, key: ScenePayloadKey) {
        if self.ready_upload_membership.insert(key) {
            self.ready_for_upload.push_front(key);
        }
    }

    pub(super) fn evict_to_budget(&mut self) {
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
            record.state = super::PayloadResidencyState::Unloaded;
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

    pub(super) fn ensure_capacity(&mut self, cpu_bytes: u64, gpu_bytes: u64) -> bool {
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
