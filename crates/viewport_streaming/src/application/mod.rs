//! Shared application bus between the Bevy viewport and WebRTC sessions.

mod interface;
mod state;
mod sync;
mod types;

pub use interface::RenderServerInterface;
pub(crate) use types::MAX_PENDING_MESSAGES;
pub use types::{
    RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind,
};

#[cfg(test)]
mod tests;
