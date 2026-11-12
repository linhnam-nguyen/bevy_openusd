//! Bounded worker boundary for composed Scene inspection.

use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
};

use project_protocol::{
    ProjectImportPhase, ProjectImportProgress, ProjectSceneInspectionResult, ProjectWriteError,
    ProjectWriteErrorCode,
};

use super::ProjectImportProgressStore;

#[derive(Clone, Debug)]
struct InspectionJob {
    operation_id: String,
    generation: u64,
    source: PathBuf,
    reply: mpsc::Sender<ProjectSceneInspectionResult>,
}

#[derive(Default)]
struct InspectionState {
    pending: Option<InspectionJob>,
}

/// One worker plus one replaceable pending job. A newer source selection
/// supersedes the pending request and receives a typed stale result.
#[derive(Clone)]
pub struct ProjectSceneInspectionQueue {
    state: Arc<(Mutex<InspectionState>, Condvar)>,
    progress: ProjectImportProgressStore,
}

impl Default for ProjectSceneInspectionQueue {
    fn default() -> Self {
        Self::with_progress(ProjectImportProgressStore::default())
    }
}

impl ProjectSceneInspectionQueue {
    pub fn with_progress(progress: ProjectImportProgressStore) -> Self {
        let state = Arc::new((Mutex::new(InspectionState::default()), Condvar::new()));
        let worker_state = Arc::clone(&state);
        let worker_progress = progress.clone();
        std::thread::Builder::new()
            .name("usdhub-scene-inspection".to_owned())
            .spawn(move || worker_loop(worker_state, worker_progress))
            .expect("Scene inspection worker must start");
        Self { state, progress }
    }

    pub fn inspect(
        &self,
        operation_id: String,
        generation: u64,
        source: PathBuf,
    ) -> ProjectSceneInspectionResult {
        let (reply, receiver) = mpsc::channel();
        let job = InspectionJob {
            operation_id,
            generation,
            source,
            reply,
        };
        self.progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase: ProjectImportPhase::Queued,
        });
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("Scene inspection state is not poisoned");
        if let Some(previous) = state.pending.replace(job) {
            let operation_id = previous.operation_id;
            let generation = previous.generation;
            let _ = previous.reply.send(ProjectSceneInspectionResult {
                operation_id: operation_id.clone(),
                generation,
                progress: ProjectImportProgress {
                    operation_id,
                    generation,
                    phase: ProjectImportPhase::Failed,
                },
                inspection: Err(ProjectWriteError::ConcurrentChange),
            });
        }
        wake.notify_one();
        drop(state);
        receiver
            .recv()
            .expect("Scene inspection worker must return a result")
    }
}

fn worker_loop(
    state: Arc<(Mutex<InspectionState>, Condvar)>,
    progress: ProjectImportProgressStore,
) -> ! {
    loop {
        let job = {
            let (lock, wake) = &*state;
            let mut state = lock.lock().expect("Scene inspection state is not poisoned");
            while state.pending.is_none() {
                state = wake
                    .wait(state)
                    .expect("Scene inspection worker wait must not be poisoned");
            }
            state.pending.take().expect("pending job exists")
        };
        progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase: ProjectImportPhase::Inspecting,
        });
        let inspection = crate::project::scene::inspection::inspect_composition(&job.source)
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            });
        let phase = if inspection.is_ok() {
            ProjectImportPhase::Inspecting
        } else {
            ProjectImportPhase::Failed
        };
        progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase,
        });
        let _ = job.reply.send(ProjectSceneInspectionResult {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            progress: ProjectImportProgress {
                operation_id: job.operation_id,
                generation: job.generation,
                phase,
            },
            inspection,
        });
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/service/scene_inspection_tests.rs"]
mod tests;
