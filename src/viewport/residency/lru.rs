//! Byte-weighted warm residency ordering.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

#[derive(Clone, Debug)]
struct WarmEntry {
    cpu_bytes: u64,
    gpu_bytes: u64,
    stamp: u64,
    pinned: bool,
}

#[derive(Debug)]
pub(crate) struct WarmLru<K> {
    entries: HashMap<K, WarmEntry>,
    order: BTreeSet<(u64, K)>,
    unpinned: BTreeSet<(u64, K)>,
    clock: u64,
}

impl<K> Default for WarmLru<K>
where
    K: Clone + Eq + Hash + Ord,
{
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeSet::new(),
            unpinned: BTreeSet::new(),
            clock: 0,
        }
    }
}

impl<K> WarmLru<K>
where
    K: Clone + Eq + Hash + Ord,
{
    pub(crate) fn insert(&mut self, key: K, cpu_bytes: u64, gpu_bytes: u64, pinned: bool) {
        self.remove(&key);
        self.clock = self.clock.saturating_add(1);
        let stamp = self.clock;
        self.order.insert((stamp, key.clone()));
        if !pinned {
            self.unpinned.insert((stamp, key.clone()));
        }
        self.entries.insert(
            key,
            WarmEntry {
                cpu_bytes,
                gpu_bytes,
                stamp,
                pinned,
            },
        );
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<(u64, u64)> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&(entry.stamp, key.clone()));
        if !entry.pinned {
            self.unpinned.remove(&(entry.stamp, key.clone()));
        }
        Some((entry.cpu_bytes, entry.gpu_bytes))
    }

    pub(crate) fn pop_oldest_unpinned(&mut self) -> Option<(K, u64, u64)> {
        let (_, key) = self.unpinned.first()?.clone();
        let candidate = (
            key.clone(),
            self.entries.get(&key)?.cpu_bytes,
            self.entries.get(&key)?.gpu_bytes,
        );
        self.remove(&candidate.0)
            .map(|(cpu, gpu)| (candidate.0, cpu, gpu))
    }

    pub(crate) fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.unpinned.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_lru_evicts_oldest_unpinned_entry() {
        let mut lru = WarmLru::default();
        lru.insert(1, 10, 20, false);
        lru.insert(2, 30, 40, true);
        lru.insert(3, 50, 60, false);
        assert_eq!(lru.pop_oldest_unpinned(), Some((1, 10, 20)));
        assert!(lru.contains(&2));
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn replacing_an_entry_refreshes_its_recency() {
        let mut lru = WarmLru::default();
        lru.insert(1, 1, 1, false);
        lru.insert(2, 1, 1, false);
        lru.insert(1, 1, 1, false);
        assert_eq!(lru.pop_oldest_unpinned().map(|entry| entry.0), Some(2));
    }
}
