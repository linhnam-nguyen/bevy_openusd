//! Recreation candidate values.

use usd_model::EntityKey;

/// A possible recreation linking one removed entity to one added entity.
///
/// This is descriptive only; the diff engine never changes the entities'
/// `Added`/`Removed` presence states based on a candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecreationCandidate {
    pub removed: EntityKey,
    pub added: EntityKey,
    pub score: u16,
    pub reasons: Vec<RecreationReason>,
}

/// Evidence used by a future recreation matcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecreationReason {
    SameCategory,
    SameFamily,
    SameType,
    SimilarTransform,
    SimilarGeometry,
}
