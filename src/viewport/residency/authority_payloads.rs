use super::*;

impl ResidencyAuthority {
    pub(crate) fn requested_mask(&self, key: &ScenePayloadKey) -> PayloadLoadMask {
        self.records
            .get(key)
            .map_or(PayloadLoadMask::default(), |record| record.requested_mask)
    }

    pub(crate) fn satisfied_mask(&self, key: &ScenePayloadKey) -> PayloadLoadMask {
        self.records
            .get(key)
            .map_or(PayloadLoadMask::default(), |record| record.satisfied_mask)
    }

    pub(crate) fn in_flight_mask(&self, key: &ScenePayloadKey) -> PayloadLoadMask {
        self.records
            .get(key)
            .map_or(PayloadLoadMask::default(), |record| record.in_flight_mask)
    }

    pub(crate) fn queued_mask(&self, key: &ScenePayloadKey) -> PayloadLoadMask {
        self.records
            .get(key)
            .map_or(PayloadLoadMask::default(), |record| record.queued_mask)
    }

    pub(super) fn enqueue_missing_load(&mut self, key: ScenePayloadKey) -> bool {
        let required = self.load_mask(&key);
        let Some((generation, satisfied, queued, in_flight, cpu_bytes, gpu_bytes)) = self
            .records
            .get(&key)
            .map(|record| {
                (
                    record.generation,
                    record.satisfied_mask,
                    record.queued_mask,
                    record.in_flight_mask,
                    record.cpu_bytes,
                    record.gpu_bytes,
                )
            })
        else {
            return false;
        };
        let covered = satisfied.union(queued).union(in_flight);
        let missing = required.missing_from(covered);
        if missing.is_empty() {
            self.pending_loads.remove(&key);
            return true;
        }
        if !in_flight.is_empty() {
            self.pending_loads.insert(key);
            return true;
        }
        if !queued.is_empty() {
            if let Some(record) = self.records.get_mut(&key) {
                record.queued_mask = queued.union(missing);
            }
            return true;
        }
        if self.terminal_failures.contains(&(key, generation)) {
            return false;
        }
        if !self.loader.enqueue(LoadJob {
            key,
            generation,
            cpu_bytes,
            gpu_bytes,
        }) {
            self.pending_loads.insert(key);
            return false;
        }
        self.pending_loads.remove(&key);
        if let Some(record) = self.records.get_mut(&key) {
            record.queued_mask = missing;
            if record.state == PayloadResidencyState::Unloaded {
                record.state = PayloadResidencyState::Queued;
            }
        }
        true
    }

    pub(crate) fn retry_pending_loads(&mut self) {
        let mut keys = self.pending_loads.iter().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        for key in keys {
            let _ = self.enqueue_missing_load(key);
        }
    }
}
