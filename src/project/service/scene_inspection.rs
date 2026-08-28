//! Bounded worker boundary for composed Scene inspection.

use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
};

use project_protocol::{
    ProjectImportPhase, ProjectImportProgress, ProjectSceneInspectionResult, ProjectWriteError,
    ProjectWriteErrorCode,
};

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
}

impl Default for ProjectSceneInspectionQueue {
    fn default() -> Self {
        let state = Arc::new((Mutex::new(InspectionState::default()), Condvar::new()));
        let worker_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("usdhub-scene-inspection".to_owned())
            .spawn(move || worker_loop(worker_state))
            .expect("Scene inspection worker must start");
        Self { state }
    }
}

impl ProjectSceneInspectionQueue {
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

fn worker_loop(state: Arc<(Mutex<InspectionState>, Condvar)>) -> ! {
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
        let inspection = crate::project::scene::inspection::inspect_composition(&job.source)
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            });
        let phase = if inspection.is_ok() {
            ProjectImportPhase::Inspecting
        } else {
            ProjectImportPhase::Failed
        };
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
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn worker_returns_owned_inspection_for_a_valid_usd_source() {
        let directory = tempfile::tempdir().expect("inspection source directory");
        let source = directory.path().join("assembly.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n    defaultPrim = \"Root\"\n)\ndef Xform \"Root\" {}\n",
        )
        .expect("write inspection source");

        let queue = ProjectSceneInspectionQueue::default();
        let result = queue.inspect("operation-1".to_owned(), 7, source);

        assert_eq!(result.operation_id, "operation-1");
        assert_eq!(result.generation, 7);
        assert_eq!(result.progress.phase, ProjectImportPhase::Inspecting);
        let inspection = result.inspection.expect("valid USD should inspect");
        assert!(!inspection.diagnostics.is_empty() || inspection.dependencies.is_empty());
    }
}
