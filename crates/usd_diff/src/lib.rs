//! Pure semantic comparison for [`usd_model`] snapshots.
//!
//! This crate deliberately has no dependency on Bevy, OpenUSD, Turso, or Git.
//! It compares stable snapshot values and leaves runtime projection, storage,
//! and history loading to their owning layers.

mod classification;
mod config;
mod engine;
mod metadata;
pub mod recreation;

pub use config::DiffConfig;
pub use engine::{DiffSummary, EntityDiff, StageDiff, compare, compare_with_config};
pub use metadata::{MetadataChange, metadata_changes};
pub use recreation::RecreationCandidate;
