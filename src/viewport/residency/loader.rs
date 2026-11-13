//! Bounded, generation-aware payload admission.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoadJob<K> {
    pub(crate) key: K,
    pub(crate) generation: u64,
    pub(crate) cpu_bytes: u64,
    pub(crate) gpu_bytes: u64,
}

/// A latest-wins queue. Repeated requests for a payload replace its pending
/// metadata without consuming another queue slot.
#[derive(Debug)]
pub(crate) struct BoundedLoader<K> {
    capacity: usize,
    order: VecDeque<K>,
    jobs: HashMap<K, LoadJob<K>>,
}

impl<K> BoundedLoader<K>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            jobs: HashMap::new(),
        }
    }

    pub(crate) fn enqueue(&mut self, job: LoadJob<K>) -> bool {
        if self.jobs.contains_key(&job.key) {
            self.jobs.insert(job.key.clone(), job);
            return true;
        }
        if self.jobs.len() >= self.capacity {
            return false;
        }
        self.order.push_back(job.key.clone());
        self.jobs.insert(job.key.clone(), job);
        true
    }

    pub(crate) fn pop(&mut self) -> Option<LoadJob<K>> {
        while let Some(key) = self.order.pop_front() {
            if let Some(job) = self.jobs.remove(&key) {
                return Some(job);
            }
        }
        None
    }

    pub(crate) fn requeue_front_all(&mut self, jobs: &mut Vec<LoadJob<K>>) {
        while let Some(job) = jobs.pop() {
            debug_assert!(!self.jobs.contains_key(&job.key));
            debug_assert!(self.jobs.len() < self.capacity);
            self.order.push_front(job.key.clone());
            self.jobs.insert(job.key.clone(), job);
        }
    }

    pub(crate) fn peek(&self) -> Option<&LoadJob<K>> {
        self.order.iter().find_map(|key| self.jobs.get(key))
    }

    pub(crate) fn cancel(&mut self, key: &K) -> Option<LoadJob<K>> {
        let removed = self.jobs.remove(key)?;
        let position = self.order.iter().position(|queued| queued == key);
        debug_assert!(position.is_some());
        if let Some(position) = position {
            let _ = self.order.remove(position);
        }
        Some(removed)
    }

    pub(crate) fn len(&self) -> usize {
        self.jobs.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.order.clear();
        self.jobs.clear();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/loader_inline_tests.rs"]
mod tests;
