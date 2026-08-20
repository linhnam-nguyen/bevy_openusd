//! Deterministic semantic extraction from an OpenUSD stage.
//!
//! The extractor reads the composed [`openusd::usd::Stage`] directly and
//! returns renderer-neutral [`usd_model`] snapshots. It deliberately does not
//! depend on Bevy or on the viewer's projection layer.

mod config;
mod extractor;
mod geometry;
mod identity;
mod metadata;
mod transform;

pub use config::{IdentityConfig, SemanticConfig};
pub use extractor::{SemanticExtractor, extract_stage};
pub use identity::resolve_identity;
