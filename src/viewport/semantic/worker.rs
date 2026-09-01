use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use bevy::prelude::Resource;
use usd_model::SemanticSnapshot;

use super::mailbox::{
    MailboxSubmitError, RESPONSE_MAILBOX_CAPACITY, SemanticMailbox, SemanticMailboxCommand,
    SemanticQueryCommand, SemanticStateCommand,
};
use super::query::SemanticQuery;
use super::store::SemanticDatabase;
use super::types::{SemanticIncrementalUpdate, SemanticResponse};

/// The Bevy-facing endpoint for the dedicated semantic worker.
#[derive(Resource, Debug)]
pub(crate) struct SemanticWorkingStore {
    mailbox: Arc<SemanticMailbox>,
    responses: Mutex<mpsc::Receiver<SemanticResponse>>,
    test_control: Arc<SemanticWorkerTestControl>,
}

/// Controlled worker behavior used only by isolation benchmark scenarios.
#[derive(Debug, Default)]
pub(crate) struct SemanticWorkerTestControl {
    query_delay_ms: std::sync::atomic::AtomicU64,
    fail_queries: std::sync::atomic::AtomicBool,
}

impl Default for SemanticWorkingStore {
    fn default() -> Self {
        let mailbox = Arc::new(SemanticMailbox::new());
        let (response_sender, response_receiver) = mpsc::sync_channel(RESPONSE_MAILBOX_CAPACITY);
        let test_control = Arc::new(SemanticWorkerTestControl::default());
        let worker_control = Arc::clone(&test_control);
        let worker_mailbox = Arc::clone(&mailbox);
        std::thread::Builder::new()
            .name("usdview-semantic-worker".to_owned())
            .spawn(move || semantic_worker(worker_mailbox, response_sender, worker_control))
            .expect("semantic worker should start");
        Self {
            mailbox,
            responses: Mutex::new(response_receiver),
            test_control,
        }
    }
}

impl SemanticWorkingStore {
    pub(crate) fn submit_snapshot(
        &self,
        request_id: impl Into<String>,
        snapshot: impl Into<Arc<SemanticSnapshot>>,
    ) -> bool {
        self.mailbox
            .submit_snapshot(request_id.into(), snapshot.into())
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
        self.mailbox
            .submit_query(request_id.into(), query)
            .map_err(SemanticSubmitError::from)
    }

    pub(crate) fn submit_delta(
        &self,
        request_id: impl Into<String>,
        update: SemanticIncrementalUpdate,
    ) -> bool {
        self.mailbox.submit_delta(request_id.into(), update).is_ok()
    }

    /// Submit a delta with its complete post-delta snapshot as a saturation
    /// recovery fallback. The snapshot is cloned only if the state mailbox is full.
    pub(crate) fn submit_delta_with_snapshot(
        &self,
        request_id: impl Into<String>,
        update: SemanticIncrementalUpdate,
        snapshot: Arc<SemanticSnapshot>,
    ) -> bool {
        self.mailbox
            .submit_delta_with_snapshot(request_id.into(), update, snapshot)
            .is_ok()
    }

    pub(crate) fn configure_test_mode(&self, query_delay: Duration, fail_queries: bool) {
        self.test_control.query_delay_ms.store(
            query_delay.as_millis().min(u64::MAX as u128) as u64,
            std::sync::atomic::Ordering::Release,
        );
        self.test_control
            .fail_queries
            .store(fail_queries, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn query_queue_high_water(&self) -> u64 {
        self.mailbox.stats().4
    }

    pub(crate) fn mailbox_stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        self.mailbox.stats()
    }

    pub(crate) fn drain_responses(&self) -> Vec<SemanticResponse> {
        let Ok(responses) = self.responses.lock() else {
            return Vec::new();
        };
        responses.try_iter().collect()
    }
}

impl Drop for SemanticWorkingStore {
    fn drop(&mut self) {
        self.mailbox.close();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticSubmitError {
    QueueFull,
    WorkerClosed,
}

impl From<MailboxSubmitError> for SemanticSubmitError {
    fn from(error: MailboxSubmitError) -> Self {
        match error {
            MailboxSubmitError::QueueFull => Self::QueueFull,
            MailboxSubmitError::Closed => Self::WorkerClosed,
        }
    }
}

fn semantic_worker(
    mailbox: Arc<SemanticMailbox>,
    responses: mpsc::SyncSender<SemanticResponse>,
    test_control: Arc<SemanticWorkerTestControl>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("semantic worker runtime should build");
    let mut database = runtime.block_on(SemanticDatabase::open()).ok();

    loop {
        let Some(command) = mailbox.recv() else {
            break;
        };
        let command = match command {
            SemanticMailboxCommand::Query(mut query) => {
                while let Some(newer) = mailbox.take_next_query() {
                    query = newer;
                }
                SemanticMailboxCommand::Query(query)
            }
            command => command,
        };

        let (request_id, result, operation) = match command {
            SemanticMailboxCommand::State(SemanticStateCommand::ReplaceSnapshot {
                request_id,
                snapshot,
            }) => {
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
            SemanticMailboxCommand::State(SemanticStateCommand::ApplyDelta {
                request_id,
                update,
            }) => {
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
            SemanticMailboxCommand::Query(SemanticQueryCommand { request_id, query }) => {
                let delay_ms = test_control
                    .query_delay_ms
                    .load(std::sync::atomic::Ordering::Acquire);
                if delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                let result = database.as_ref().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        if test_control
                            .fail_queries
                            .load(std::sync::atomic::Ordering::Acquire)
                        {
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
