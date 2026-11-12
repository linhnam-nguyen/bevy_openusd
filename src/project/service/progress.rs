//! Bounded backend-owned status for long-running Project imports.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use project_protocol::ProjectImportProgress;

const PROGRESS_CAPACITY: usize = 64;
type ProgressKey = (String, u64);

#[derive(Default)]
struct ProgressState {
    latest: HashMap<ProgressKey, ProjectImportProgress>,
    order: VecDeque<ProgressKey>,
}

/// Coalesced status store shared by Project workers and the native host
/// commands. It retains only the latest phase for each operation generation.
#[derive(Clone, Default)]
pub struct ProjectImportProgressStore {
    state: Arc<Mutex<ProgressState>>,
}

impl ProjectImportProgressStore {
    pub fn publish(&self, progress: ProjectImportProgress) {
        let key = (progress.operation_id.clone(), progress.generation);
        let mut state = self
            .state
            .lock()
            .expect("Project import progress state is not poisoned");
        state.order.retain(|existing| existing != &key);
        state.order.push_back(key.clone());
        state.latest.insert(key, progress);
        while state.order.len() > PROGRESS_CAPACITY {
            let Some(expired) = state.order.pop_front() else {
                break;
            };
            state.latest.remove(&expired);
        }
    }

    pub fn latest(&self, operation_id: &str, generation: u64) -> Option<ProjectImportProgress> {
        self.state
            .lock()
            .expect("Project import progress state is not poisoned")
            .latest
            .get(&(operation_id.to_owned(), generation))
            .cloned()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/service/progress_tests.rs"]
mod tests;
