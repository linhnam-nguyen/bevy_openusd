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
mod nvidia;
mod transform;
pub mod units;

pub use config::{IdentityConfig, SemanticConfig};
pub use extractor::{SemanticExtractor, extract_stage};
pub use identity::resolve_identity;
pub use nvidia::{NvidiaRevitConfig, NvidiaRevitIdentityConfig, NvidiaRevitMeasurementMapping};
pub use units::{UnitConversionError, UnitDefinition, UnitRegistry};
