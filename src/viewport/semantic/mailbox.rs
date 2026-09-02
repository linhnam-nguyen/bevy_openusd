use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use usd_model::{SemanticSnapshot, SnapshotSource};

use super::query::SemanticQuery;
use super::types::SemanticIncrementalUpdate;

pub(crate) const STATE_MAILBOX_CAPACITY: usize = 8;
pub(crate) const QUERY_MAILBOX_CAPACITY: usize = 8;
pub(crate) const RESPONSE_MAILBOX_CAPACITY: usize = 32;

#[derive(Debug)]
pub(crate) enum SemanticStateCommand {
    ReplaceSnapshot {
        request_id: String,
        snapshot: Arc<SemanticSnapshot>,
    },
    ApplyDelta {
        request_id: String,
        update: SemanticIncrementalUpdate,
    },
}

#[derive(Debug)]
pub(crate) struct SemanticQueryCommand {
    pub(crate) request_id: String,
    pub(crate) query: SemanticQuery,
}

#[derive(Debug)]
pub(crate) enum SemanticMailboxCommand {
    State(SemanticStateCommand),
    Query(SemanticQueryCommand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MailboxSubmitError {
    QueueFull,
    Closed,
}

#[derive(Debug, Default)]
struct MailboxState {
    commands: VecDeque<SemanticMailboxCommand>,
    state_count: usize,
    query_count: usize,
    closed: bool,
    state_high_water: u64,
    query_high_water: u64,
    state_recoveries: u64,
    query_coalesced: u64,
}

/// Bounded semantic state/query mailboxes with a complete-snapshot recovery lane.
#[derive(Debug)]
pub(crate) struct SemanticMailbox {
    state: Mutex<MailboxState>,
    wake: Condvar,
}

impl SemanticMailbox {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(MailboxState::default()),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn submit_snapshot(
        &self,
        request_id: String,
        snapshot: Arc<SemanticSnapshot>,
    ) -> Result<bool, MailboxSubmitError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(MailboxSubmitError::Closed);
        };
        if state.closed {
            return Err(MailboxSubmitError::Closed);
        }
        let recovered = state.state_count >= STATE_MAILBOX_CAPACITY;
        if recovered {
            self.replace_with_snapshot(&mut state, request_id, snapshot);
        } else {
            state.commands.push_back(SemanticMailboxCommand::State(
                SemanticStateCommand::ReplaceSnapshot {
                    request_id,
                    snapshot,
                },
            ));
            state.state_count += 1;
            self.update_state_high_water(&mut state);
        }
        self.wake.notify_one();
        Ok(recovered)
    }

    pub(crate) fn submit_delta(
        &self,
        request_id: String,
        update: SemanticIncrementalUpdate,
    ) -> Result<(), MailboxSubmitError> {
        self.submit_delta_inner(request_id, update, None)
            .map(|_| ())
    }

    pub(crate) fn submit_delta_with_snapshot(
        &self,
        request_id: String,
        update: SemanticIncrementalUpdate,
        snapshot: Arc<SemanticSnapshot>,
    ) -> Result<bool, MailboxSubmitError> {
        self.submit_delta_inner(request_id, update, Some(snapshot))
    }

    fn submit_delta_inner(
        &self,
        request_id: String,
        update: SemanticIncrementalUpdate,
        recovery_snapshot: Option<Arc<SemanticSnapshot>>,
    ) -> Result<bool, MailboxSubmitError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(MailboxSubmitError::Closed);
        };
        if state.closed {
            return Err(MailboxSubmitError::Closed);
        }
        if state.state_count >= STATE_MAILBOX_CAPACITY {
            let Some(snapshot) = recovery_snapshot else {
                return Err(MailboxSubmitError::QueueFull);
            };
            self.replace_with_snapshot(&mut state, request_id, snapshot);
            self.wake.notify_one();
            return Ok(true);
        }
        state.commands.push_back(SemanticMailboxCommand::State(
            SemanticStateCommand::ApplyDelta { request_id, update },
        ));
        state.state_count += 1;
        self.update_state_high_water(&mut state);
        self.wake.notify_one();
        Ok(false)
    }

    pub(crate) fn submit_query(
        &self,
        request_id: String,
        query: SemanticQuery,
    ) -> Result<(), MailboxSubmitError> {
        let Ok(mut state) = self.state.lock() else {
            return Err(MailboxSubmitError::Closed);
        };
        if state.closed {
            return Err(MailboxSubmitError::Closed);
        }
        if state.query_count >= QUERY_MAILBOX_CAPACITY {
            return Err(MailboxSubmitError::QueueFull);
        }
        state
            .commands
            .push_back(SemanticMailboxCommand::Query(SemanticQueryCommand {
                request_id,
                query,
            }));
        state.query_count += 1;
        state.query_high_water = state.query_high_water.max(state.query_count as u64);
        self.wake.notify_one();
        Ok(())
    }

    pub(crate) fn recv(&self) -> Option<SemanticMailboxCommand> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(command) = state.commands.pop_front() {
                match command {
                    SemanticMailboxCommand::State(_) => state.state_count -= 1,
                    SemanticMailboxCommand::Query(_) => state.query_count -= 1,
                }
                return Some(command);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    pub(crate) fn take_next_query(&self) -> Option<SemanticQueryCommand> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if !matches!(
            state.commands.front(),
            Some(SemanticMailboxCommand::Query(_))
        ) {
            return None;
        }
        let Some(SemanticMailboxCommand::Query(query)) = state.commands.pop_front() else {
            return None;
        };
        state.query_count -= 1;
        state.query_coalesced += 1;
        Some(query)
    }

    pub(crate) fn stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        self.state.lock().map_or((0, 0, 0, 0, 0, 0), |state| {
            (
                state.state_count as u64,
                state.state_high_water,
                state.state_recoveries,
                state.query_count as u64,
                state.query_high_water,
                state.query_coalesced,
            )
        })
    }

    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.wake.notify_all();
        }
    }

    fn replace_with_snapshot(
        &self,
        state: &mut MailboxState,
        request_id: String,
        snapshot: Arc<SemanticSnapshot>,
    ) {
        state.commands.clear();
        state.state_count = 0;
        state.query_count = 0;
        state.state_recoveries += 1;
        state.commands.push_back(SemanticMailboxCommand::State(
            SemanticStateCommand::ReplaceSnapshot {
                request_id,
                snapshot,
            },
        ));
        state.state_count = 1;
        self.update_state_high_water(state);
    }

    fn update_state_high_water(&self, state: &mut MailboxState) {
        state.state_high_water = state.state_high_water.max(state.state_count as u64);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use usd_model::{HashDigest, SemanticSnapshot, SnapshotId};

    use super::*;

    fn snapshot() -> SemanticSnapshot {
        SemanticSnapshot {
            snapshot_id: SnapshotId("test".to_owned()),
            source: SnapshotSource::Working {
                session: "test".to_owned(),
                live_revision: 9,
            },
            config_hash: HashDigest::new([0; 32]),
            entities: HashMap::new(),
        }
    }

    fn update(revision: u64) -> SemanticIncrementalUpdate {
        SemanticIncrementalUpdate {
            snapshot_id: SnapshotId("test".to_owned()),
            source: SnapshotSource::Working {
                session: "test".to_owned(),
                live_revision: revision,
            },
            config_hash: HashDigest::new([0; 32]),
            upserts: Vec::new(),
            removed_paths: Vec::new(),
        }
    }

    #[test]
    fn state_saturation_replaces_pending_deltas_with_latest_snapshot() {
        let mailbox = SemanticMailbox::new();
        for revision in 1..=STATE_MAILBOX_CAPACITY {
            assert_eq!(
                mailbox
                    .submit_delta(format!("delta-{revision}"), update(revision as u64))
                    .unwrap(),
                ()
            );
        }
        assert!(
            mailbox
                .submit_delta_with_snapshot(
                    "recovery".to_owned(),
                    update(99),
                    Arc::new(snapshot()),
                )
                .unwrap()
        );
        let command = mailbox.recv().expect("recovery command");
        assert!(matches!(
            command,
            SemanticMailboxCommand::State(SemanticStateCommand::ReplaceSnapshot { request_id, .. })
                if request_id == "recovery"
        ));
        assert_eq!(mailbox.stats().2, 1);
    }
}
