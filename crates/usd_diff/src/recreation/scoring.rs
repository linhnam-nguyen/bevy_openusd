//! Placeholder for the later recreation heuristic.

use super::RecreationCandidate;
use usd_model::EntitySnapshot;

/// Recreation matching is intentionally not enabled by Milestone 8.
#[allow(dead_code)]
pub(crate) fn score_recreation(
    _removed: &EntitySnapshot,
    _added: &EntitySnapshot,
) -> Option<RecreationCandidate> {
    None
}
