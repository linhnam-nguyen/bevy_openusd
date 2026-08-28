//! Bounded worker boundary for Model preparation.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

use project_protocol::{ProjectModelPreparationResult, ProjectWriteError, ProjectWriteErrorCode};

use crate::project::model_import::{ModelImportRequest, ModelImporterRegistry, PreparedModel};

struct PreparationJob {
    operation_id: String,
    generation: u64,
    source: PathBuf,
    reply: mpsc::Sender<ProjectModelPreparationResult>,
}

/// One bounded preparation worker. The prepared backend artifact stays here
/// until the host asks the same operation to publish it.
#[derive(Clone)]
pub struct ProjectModelPreparationQueue {
    sender: mpsc::SyncSender<PreparationJob>,
    prepared: Arc<Mutex<HashMap<(String, u64), PreparedModel>>>,
}

impl Default for ProjectModelPreparationQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(4);
        let prepared = Arc::new(Mutex::new(HashMap::new()));
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
                operation_id,
                generation,
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
            .remove(&(operation_id.to_owned(), generation))
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<PreparationJob>,
    prepared: Arc<Mutex<HashMap<(String, u64), PreparedModel>>>,
) -> ! {
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
        let _ = job.reply.send(ProjectModelPreparationResult {
            operation_id: job.operation_id,
            generation: job.generation,
            inspection: inspection.map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            }),
        });
    }
    unreachable!("Model preparation worker channel is retained by the queue")
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
}
