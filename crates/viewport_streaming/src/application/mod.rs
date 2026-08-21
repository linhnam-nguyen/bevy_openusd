//! Shared application bus between the Bevy viewport and WebRTC sessions.

mod interface;
mod state;
mod sync;
mod types;

pub use interface::RenderServerInterface;
pub use types::{RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind};

#[cfg(test)]
pub(crate) use types::MAX_PENDING_MESSAGES;

#[cfg(test)]
mod tests;
