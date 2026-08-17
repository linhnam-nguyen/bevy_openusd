//! Future heuristic matching for removed and added entities.
//!
//! Identity matching remains deterministic in Milestone 8. Recreation
//! candidates are intentionally kept as an explicit result type so a later
//! heuristic pass cannot silently turn a removal and an addition into a
//! modification.

mod candidate;
mod scoring;

pub use candidate::{RecreationCandidate, RecreationReason};
