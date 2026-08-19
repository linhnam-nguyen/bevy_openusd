use usd_model::SemanticSnapshot;

use super::super::types::SemanticIncrementalUpdate;

#[derive(Debug)]
pub(crate) enum SubtreeUpdateError {
    UnnormalizableRoot(String),
    EntityKeyCollision(String),
    ExtractionFailed(anyhow::Error),
}

impl std::fmt::Display for SubtreeUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnnormalizableRoot(msg) => write!(f, "unnormalizable root: {msg}"),
            Self::EntityKeyCollision(msg) => write!(f, "EntityKey collision: {msg}"),
            Self::ExtractionFailed(err) => write!(f, "subtree extraction failed: {err:#}"),
        }
    }
}

impl std::error::Error for SubtreeUpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ExtractionFailed(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for SubtreeUpdateError {
    fn from(err: anyhow::Error) -> Self {
        Self::ExtractionFailed(err)
    }
}

impl SubtreeUpdateError {
    pub(crate) fn fallback_reason(&self) -> &'static str {
        match self {
            Self::UnnormalizableRoot(_) => "unnormalizable_root",
            Self::EntityKeyCollision(_) => "semantic_entity_key_collision",
            Self::ExtractionFailed(_) => "subtree_delta_extraction_failed",
        }
    }
}

pub(crate) enum SemanticSyncAction {
    Replace(SemanticSnapshot),
    Delta(SemanticDelta),
}

pub(crate) struct SemanticDelta {
    pub(crate) request: SemanticIncrementalUpdate,
    pub(crate) snapshot: SemanticSnapshot,
}
