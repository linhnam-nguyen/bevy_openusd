//! Working semantic query service backed by an in-memory Turso database.
//!
//! Semantic rows remain renderer-neutral. The viewport bridge adapts their
//! prim paths through `SceneAnchorIndex` when publishing search results.

mod diff;
mod query;
mod state;
mod store;
pub(crate) mod sync;
mod types;
mod worker;

pub(crate) use diff::SemanticDiffState;
pub(crate) use query::{GroupField, SemanticFilter, SemanticQuery, SemanticQueryResult};
pub(crate) use state::SemanticSyncState;
pub(crate) use sync::synchronize_live_stage;
#[cfg(test)]
pub(crate) use crate::viewport::api::RenderServerInterface;
#[cfg(test)]
pub(crate) use sync::{
    SemanticDelta, SemanticSyncAction, SubtreeUpdateError, attach_render_blobs_to_action,
    changed_info_update, resync_subtree_update,
};
pub(crate) use types::{SemanticIncrementalUpdate, SemanticResponse};
pub(crate) use worker::SemanticWorkingStore;

#[cfg(test)]
mod tests;
