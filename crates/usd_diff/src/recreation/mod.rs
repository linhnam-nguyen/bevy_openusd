//! Heuristic matching for removed and added entities.
//!
//! Identity matching remains deterministic. Recreation candidates are kept as
//! an explicit result type so the heuristic cannot silently turn a removal and
//! an addition into a modification.

mod candidate;
mod scoring;

pub use candidate::{RecreationCandidate, RecreationReason};

use usd_model::EntitySnapshot;

/// Return all deterministic pairs that pass the recreation confidence floor.
///
/// Candidates are sorted from strongest to weakest and then by stable entity
/// keys so the result is reproducible across `HashMap` iteration order. The
/// result is descriptive only; callers must not convert `Removed`/`Added`
/// presence into `Existing`.
pub(crate) fn find_recreations<'a>(
    removed: impl IntoIterator<Item = &'a EntitySnapshot>,
    added: impl IntoIterator<Item = &'a EntitySnapshot>,
) -> Vec<RecreationCandidate> {
    let removed = removed.into_iter().collect::<Vec<_>>();
    let added = added.into_iter().collect::<Vec<_>>();
    let mut candidates = removed
        .iter()
        .flat_map(|removed| {
            added
                .iter()
                .filter_map(move |added| scoring::score_recreation(removed, added))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.removed.cmp(&right.removed))
            .then_with(|| left.added.cmp(&right.added))
    });
    candidates
}
