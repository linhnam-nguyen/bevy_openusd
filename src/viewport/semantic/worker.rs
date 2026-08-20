use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, SyncSender, TrySendError},
};
use std::time::Duration;

use bevy::prelude::Resource;
use usd_model::SemanticSnapshot;

use super::query::SemanticQuery;
use super::store::SemanticDatabase;
use super::types::{SemanticIncrementalUpdate, SemanticResponse};

#[derive(Debug)]
enum SemanticCommand {
    ReplaceSnapshot {
        request_id: String,
        snapshot: SemanticSnapshot,
    },
    ApplyDelta {
        request_id: String,
        update: SemanticIncrementalUpdate,
    },
    Query {
        request_id: String,
        query: SemanticQuery,
    },
}

/// The Bevy-facing channel endpoint for the dedicated semantic worker.
#[derive(Resource, Debug)]
pub(crate) struct SemanticWorkingStore {
    commands: SyncSender<SemanticCommand>,
    responses: Mutex<mpsc::Receiver<SemanticResponse>>,
    test_control: Arc<SemanticWorkerTestControl>,
}

/// Controlled worker behavior used only by the isolation benchmark scenario.
/// The delay and failure happen on the dedicated worker thread, never in a
/// Bevy system, so the scenario can prove that rendering remains responsive
/// while data-plane work is slow or failing.
#[derive(Debug, Default)]
pub(crate) struct SemanticWorkerTestControl {
    query_delay_ms: AtomicU64,
    fail_queries: AtomicBool,
}

impl Default for SemanticWorkingStore {
    fn default() -> Self {
        let (commands, pending_commands) = mpsc::sync_channel(8);
        let (responses, pending_responses) = mpsc::channel();
        let test_control = Arc::new(SemanticWorkerTestControl::default());
        let worker_control = Arc::clone(&test_control);
        std::thread::Builder::new()
            .name("usdview-semantic-worker".to_owned())
            .spawn(move || semantic_worker(pending_commands, responses, worker_control))
            .expect("semantic worker should start");
        Self {
            commands,
            responses: Mutex::new(pending_responses),
            test_control,
        }
    }
}

impl SemanticWorkingStore {
    pub(crate) fn submit_snapshot(
        &self,
        request_id: impl Into<String>,
        snapshot: SemanticSnapshot,
    ) -> bool {
        self.commands
            .try_send(SemanticCommand::ReplaceSnapshot {
                request_id: request_id.into(),
                snapshot,
            })
            .is_ok()
    }

    pub(crate) fn submit_query(&self, request_id: impl Into<String>, query: SemanticQuery) -> bool {
        self.try_submit_query(request_id, query).is_ok()
    }

    pub(crate) fn try_submit_query(
        &self,
        request_id: impl Into<String>,
        query: SemanticQuery,
    ) -> Result<(), SemanticSubmitError> {
        self.commands
            .try_send(SemanticCommand::Query {
                request_id: request_id.into(),
                query,
            })
            .map_err(SemanticSubmitError::from)
    }

    pub(crate) fn submit_delta(
        &self,
        request_id: impl Into<String>,
        update: SemanticIncrementalUpdate,
    ) -> bool {
        self.commands
            .try_send(SemanticCommand::ApplyDelta {
                request_id: request_id.into(),
                update,
            })
            .is_ok()
    }

    pub(crate) fn configure_test_mode(&self, query_delay: Duration, fail_queries: bool) {
        self.test_control
            .query_delay_ms
            .store(query_delay.as_millis().min(u64::MAX as u128) as u64, Ordering::Release);
        self.test_control
            .fail_queries
            .store(fail_queries, Ordering::Release);
    }

    pub(crate) fn drain_responses(&self) -> Vec<SemanticResponse> {
        let Ok(responses) = self.responses.lock() else {
            return Vec::new();
        };
        responses.try_iter().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticSubmitError {
    QueueFull,
    WorkerClosed,
}

impl From<TrySendError<SemanticCommand>> for SemanticSubmitError {
    fn from(error: TrySendError<SemanticCommand>) -> Self {
        match error {
            TrySendError::Full(_) => Self::QueueFull,
            TrySendError::Disconnected(_) => Self::WorkerClosed,
        }
    }
}

fn semantic_worker(
    pending_commands: mpsc::Receiver<SemanticCommand>,
    responses: mpsc::Sender<SemanticResponse>,
    test_control: Arc<SemanticWorkerTestControl>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("semantic worker runtime should build");
    let mut database = runtime.block_on(SemanticDatabase::open()).ok();
    let mut buffered_command = None;

    loop {
        let Some(command) = buffered_command
            .take()
            .or_else(|| pending_commands.recv().ok())
        else {
            break;
        };

        // Preserve the old search-worker behavior: a burst of consecutive
        // query requests can be coalesced, while snapshot/delta commands stay
        // ordered and act as barriers for the query stream.
        let command = match command {
            SemanticCommand::Query {
                mut request_id,
                mut query,
            } => {
                while let Ok(next) = pending_commands.try_recv() {
                    match next {
                        SemanticCommand::Query {
                            request_id: newer_request_id,
                            query: newer_query,
                        } => {
                            request_id = newer_request_id;
                            query = newer_query;
                        }
                        other => {
                            buffered_command = Some(other);
                            break;
                        }
                    }
                }
                SemanticCommand::Query { request_id, query }
            }
            other => other,
        };

        let (request_id, result, operation) = match command {
            SemanticCommand::ReplaceSnapshot {
                request_id,
                snapshot,
            } => {
                let result = database.as_mut().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.replace_snapshot(&snapshot))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|count| SemanticResponse::SnapshotLoaded {
                        request_id: String::new(),
                        entity_count: count,
                    }),
                    "snapshot load",
                )
            }
            SemanticCommand::ApplyDelta { request_id, update } => {
                let result = database.as_mut().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.apply_delta(&update))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|(upserted, removed)| SemanticResponse::DeltaApplied {
                        request_id: String::new(),
                        upserted,
                        removed,
                    }),
                    "semantic delta",
                )
            }
            SemanticCommand::Query { request_id, query } => {
                let delay_ms = test_control.query_delay_ms.load(Ordering::Acquire);
                if delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                let result = database.as_ref().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        if test_control.fail_queries.load(Ordering::Acquire) {
                            Err("controlled benchmark query failure".to_owned())
                        } else {
                            runtime
                                .block_on(database.query(&query))
                                .map_err(|error| error.to_string())
                        }
                    },
                );
                (
                    request_id,
                    result.map(|result| SemanticResponse::QueryResult {
                        request_id: String::new(),
                        result,
                    }),
                    "query",
                )
            }
        };

        let response = match result {
            Ok(mut response) => {
                match &mut response {
                    SemanticResponse::SnapshotLoaded {
                        request_id: response_id,
                        ..
                    }
                    | SemanticResponse::DeltaApplied {
                        request_id: response_id,
                        ..
                    }
                    | SemanticResponse::QueryResult {
                        request_id: response_id,
                        ..
                    } => *response_id = request_id,
                    SemanticResponse::Failed { .. } => {}
                }
                response
            }
            Err(error) => SemanticResponse::Failed {
                request_id,
                operation,
                error,
            },
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}
