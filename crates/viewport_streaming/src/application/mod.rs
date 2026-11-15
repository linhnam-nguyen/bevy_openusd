//! Shared application bus between the Bevy viewport and WebRTC sessions.

mod interface;
mod state;
mod sync;
mod types;

pub use interface::RenderServerInterface;
pub use types::{ProjectActivationRequest, ProjectActivationResult};
pub use types::{RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind};

#[cfg(test)]
#[path = "../../tests/unit/application/mod.rs"]
pub(crate) mod tests;
