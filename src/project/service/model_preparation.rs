//! Bounded worker boundary for Model preparation.

use std::{
    collections::HashMap,
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

use project_protocol::{
    ProjectImportPhase, ProjectImportProgress, ProjectModelPreparationResult, ProjectWriteError,
    ProjectWriteErrorCode,
};

use crate::project::model_import::{ModelImportRequest, ModelImporterRegistry, PreparedModel};

use super::ProjectImportProgressStore;

struct PreparationJob {
    operation_id: String,
    generation: u64,
    source: PathBuf,
    reply: mpsc::Sender<ProjectModelPreparationResult>,
}

const PREPARATION_CAPACITY: usize = 4;
type PreparedKey = (String, u64);

struct PreparedState {
    artifacts: HashMap<PreparedKey, PreparedModel>,
    order: VecDeque<PreparedKey>,
}

impl PreparedState {
    fn new() -> Self {
        Self {
            artifacts: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, key: PreparedKey, prepared: PreparedModel) {
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key.clone());
        self.artifacts.insert(key, prepared);
        while self.order.len() > PREPARATION_CAPACITY {
            let Some(expired) = self.order.pop_front() else {
                break;
            };
            self.artifacts.remove(&expired);
        }
    }

    fn take(&mut self, key: &PreparedKey) -> Option<PreparedModel> {
        self.order.retain(|existing| existing != key);
        self.artifacts.remove(key)
    }
}

/// One bounded preparation worker. The prepared backend artifact stays here
/// until the host asks the same operation to publish it.
#[derive(Clone)]
pub struct ProjectModelPreparationQueue {
    sender: mpsc::SyncSender<PreparationJob>,
    prepared: Arc<Mutex<PreparedState>>,
    progress: ProjectImportProgressStore,
}

impl Default for ProjectModelPreparationQueue {
    fn default() -> Self {
        Self::with_progress(ProjectImportProgressStore::default())
    }
}

impl ProjectModelPreparationQueue {
    pub fn with_progress(progress: ProjectImportProgressStore) -> Self {
        let (sender, receiver) = mpsc::sync_channel(PREPARATION_CAPACITY);
        let prepared = Arc::new(Mutex::new(PreparedState::new()));
        let worker_prepared = Arc::clone(&prepared);
        let worker_progress = progress.clone();
        std::thread::Builder::new()
            .name("usdhub-model-preparation".to_owned())
            .spawn(move || worker_loop(receiver, worker_prepared, worker_progress))
            .expect("Model preparation worker must start");
        Self {
            sender,
            prepared,
            progress,
        }
    }

    pub fn prepare(
        &self,
        operation_id: String,
        generation: u64,
        source: PathBuf,
    ) -> ProjectModelPreparationResult {
        let (reply, receiver) = mpsc::channel();
        let job = PreparationJob {
            operation_id: operation_id.clone(),
            generation,
            source,
            reply,
        };
        self.progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase: ProjectImportPhase::Queued,
        });
        if self.sender.try_send(job).is_err() {
            self.progress.publish(ProjectImportProgress {
                operation_id: operation_id.clone(),
                generation,
                phase: ProjectImportPhase::Failed,
            });
            return ProjectModelPreparationResult {
                operation_id: operation_id.clone(),
                generation,
                progress: ProjectImportProgress {
                    operation_id: operation_id.clone(),
                    generation,
                    phase: ProjectImportPhase::Failed,
                },
                inspection: Err(ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::Busy,
                }),
            };
        }
        receiver
            .recv()
            .expect("Model preparation worker must return a result")
    }

    pub(crate) fn take_prepared(
        &self,
        operation_id: &str,
        generation: u64,
    ) -> Option<PreparedModel> {
        self.prepared
            .lock()
            .expect("Model preparation state is not poisoned")
            .take(&(operation_id.to_owned(), generation))
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<PreparationJob>,
    prepared: Arc<Mutex<PreparedState>>,
    progress: ProjectImportProgressStore,
) {
    let registry = ModelImporterRegistry::default();
    let importer = registry
        .importer_for(&usd_project::ModelSourceKind::Usd)
        .expect("USD Model importer is registered");
    while let Ok(job) = receiver.recv() {
        progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase: ProjectImportPhase::Inspecting,
        });
        let inspection = importer.inspect(&job.source).and_then(|inspection| {
            progress.publish(ProjectImportProgress {
                operation_id: job.operation_id.clone(),
                generation: job.generation,
                phase: ProjectImportPhase::Preparing,
            });
            let prepared_model = importer.prepare(ModelImportRequest {
                source: job.source,
                inspection,
            })?;
            let public_inspection = prepared_model.inspection.composition.clone();
            prepared
                .lock()
                .expect("Model preparation state is not poisoned")
                .insert((job.operation_id.clone(), job.generation), prepared_model);
            Ok(public_inspection)
        });
        let inspection = inspection.map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        });
        let phase = if inspection.is_ok() {
            ProjectImportPhase::Preparing
        } else {
            ProjectImportPhase::Failed
        };
        progress.publish(ProjectImportProgress {
            operation_id: job.operation_id.clone(),
            generation: job.generation,
            phase,
        });
        let _ = job.reply.send(ProjectModelPreparationResult {
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
#[path = "../../../tests/unit/project/service/model_preparation_tests.rs"]
mod tests;
