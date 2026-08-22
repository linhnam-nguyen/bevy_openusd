//! Runtime-memory to Git commit coordination.

#[cfg(test)]
mod pipeline;
#[cfg(test)]
mod state;

#[cfg(test)]
pub(crate) use pipeline::commit_live_stage;
#[cfg(test)]
pub(crate) use state::CommitState;

#[cfg(test)]
mod tests;
