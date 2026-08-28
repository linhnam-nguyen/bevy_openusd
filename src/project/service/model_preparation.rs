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
}

impl Default for ProjectModelPreparationQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(PREPARATION_CAPACITY);
        let prepared = Arc::new(Mutex::new(PreparedState::new()));
        let worker_prepared = Arc::clone(&prepared);
        std::thread::Builder::new()
            .name("usdhub-model-preparation".to_owned())
            .spawn(move || worker_loop(receiver, worker_prepared))
            .expect("Model preparation worker must start");
        Self { sender, prepared }
    }
}

impl ProjectModelPreparationQueue {
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
        if self.sender.try_send(job).is_err() {
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

fn worker_loop(receiver: mpsc::Receiver<PreparationJob>, prepared: Arc<Mutex<PreparedState>>) {
    let registry = ModelImporterRegistry::default();
    let importer = registry
        .importer_for(&usd_project::ModelSourceKind::Usd)
        .expect("USD Model importer is registered");
    while let Ok(job) = receiver.recv() {
        let inspection = importer.inspect(&job.source).and_then(|inspection| {
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
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn bounded_worker_prepares_owned_model_metadata_without_exposing_source_path() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("asset.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
        )
        .unwrap();
        let queue = ProjectModelPreparationQueue::default();
        let result = queue.prepare("operation-1".to_owned(), 4, source);

        assert_eq!(result.operation_id, "operation-1");
        assert_eq!(result.generation, 4);
        assert!(result.inspection.is_ok());
        assert!(queue.take_prepared("operation-1", 4).is_some());
    }

    #[test]
    fn abandoned_prepared_models_are_evicted_after_the_bounded_retention_window() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("asset.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
        )
        .unwrap();
        let queue = ProjectModelPreparationQueue::default();

        for generation in 0..=PREPARATION_CAPACITY as u64 {
            let result = queue.prepare("eviction".to_owned(), generation, source.clone());
            assert!(result.inspection.is_ok());
        }

        assert!(queue.take_prepared("eviction", 0).is_none());
        assert!(
            queue
                .take_prepared("eviction", PREPARATION_CAPACITY as u64)
                .is_some()
        );
    }
}
