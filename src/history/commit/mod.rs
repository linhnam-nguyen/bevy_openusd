//! Runtime-memory to Git commit coordination.

mod pipeline;
pub(crate) mod runtime;
pub(crate) mod service;
mod state;
mod worktree;

pub(crate) use pipeline::commit_live_stage;
pub(crate) use state::{CommitLease, CommitState};

#[cfg(test)]
#[path = "../../../tests/unit/project/versioning/commit/tests.rs"]
mod tests;
