//! Runtime-memory to Git commit coordination.

mod pipeline;
mod state;

pub(crate) use pipeline::commit_live_stage;
pub(crate) use state::{CommitOutcome, CommitState};

#[cfg(test)]
mod tests;
