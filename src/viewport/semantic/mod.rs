//! Working semantic query service backed by an in-memory Turso database.
//!
//! Semantic rows remain renderer-neutral. The viewport bridge adapts their
//! prim paths through `SceneAnchorIndex` when publishing search results.

mod diff;
mod query;
mod state;
mod store;
mod sync;
mod types;
mod worker;

#[cfg(test)]
pub(in crate::viewport::semantic) use crate::viewport::api::RenderServerInterface;
pub(crate) use diff::SemanticDiffState;
pub(crate) use query::{GroupField, SemanticFilter, SemanticQuery, SemanticQueryResult};
pub(crate) use state::SemanticSyncState;
#[cfg(test)]
pub(in crate::viewport::semantic) use sync::{
    SemanticDelta, SemanticSyncAction, attach_render_blobs_to_action, changed_info_update,
    resync_subtree_update,
};
pub(crate) use sync::{SubtreeUpdateError, synchronize_live_stage};
pub(crate) use types::{SemanticIncrementalUpdate, SemanticResponse};
pub(crate) use worker::SemanticWorkingStore;

#[cfg(test)]
mod tests;
