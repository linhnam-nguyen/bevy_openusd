//! Deterministic recreation candidate scoring.

use super::{RecreationCandidate, RecreationReason};
use usd_model::EntitySnapshot;

const MIN_RECREATION_SCORE: u16 = 45;
const TRANSLATION_TOLERANCE_MM: u64 = 250;
const ROTATION_TOLERANCE: u32 = 2_500;
const SCALE_TOLERANCE: u32 = 500;

/// Score one removed/added pair without changing either entity's presence.
///
/// A candidate needs at least two independent reasons. This prevents a common
/// geometry or category from turning every one-sided entity into a likely
/// recreation while still allowing a same-shape object at the old position to
/// score highly.
pub(crate) fn score_recreation(
    removed: &EntitySnapshot,
    added: &EntitySnapshot,
) -> Option<RecreationCandidate> {
    let mut score = 0;
    let mut reasons = Vec::with_capacity(5);

    if same_value(
        removed.semantic.category.as_deref(),
        added.semantic.category.as_deref(),
    ) {
        score += 10;
        reasons.push(RecreationReason::SameCategory);
    }
    if same_value(
        removed.semantic.family.as_deref(),
        added.semantic.family.as_deref(),
    ) {
        score += 15;
        reasons.push(RecreationReason::SameFamily);
    }
    if same_type(removed, added) {
        score += 20;
        reasons.push(RecreationReason::SameType);
    }
    if similar_transform(removed, added) {
        score += 20;
        reasons.push(RecreationReason::SimilarTransform);
    }
    if similar_geometry(removed, added) {
        score += 35;
        reasons.push(RecreationReason::SimilarGeometry);
    }

    (reasons.len() >= 2 && score >= MIN_RECREATION_SCORE).then_some(RecreationCandidate {
        removed: removed.key.clone(),
        added: added.key.clone(),
        score,
        reasons,
    })
}

fn same_value(old: Option<&str>, new: Option<&str>) -> bool {
    matches!((old, new), (Some(old), Some(new)) if !old.is_empty() && old == new)
}

fn same_type(removed: &EntitySnapshot, added: &EntitySnapshot) -> bool {
    same_value(
        removed.semantic.type_id.as_deref(),
        added.semantic.type_id.as_deref(),
    ) || same_value(
        removed.semantic.type_name.as_deref(),
        added.semantic.type_name.as_deref(),
    )
}

fn similar_transform(removed: &EntitySnapshot, added: &EntitySnapshot) -> bool {
    removed
        .transform
        .translation_mm
        .iter()
        .zip(added.transform.translation_mm)
        .all(|(old, new)| old.abs_diff(new) <= TRANSLATION_TOLERANCE_MM)
        && removed
            .transform
            .rotation_quantized
            .iter()
            .zip(added.transform.rotation_quantized)
            .all(|(old, new)| old.abs_diff(new) <= ROTATION_TOLERANCE)
        && removed
            .transform
            .scale_quantized
            .iter()
            .zip(added.transform.scale_quantized)
            .all(|(old, new)| old.abs_diff(new) <= SCALE_TOLERANCE)
}

fn similar_geometry(removed: &EntitySnapshot, added: &EntitySnapshot) -> bool {
    let (Some(old), Some(new)) = (removed.geometry.as_ref(), added.geometry.as_ref()) else {
        return false;
    };

    old.vertex_count == new.vertex_count
        && old.index_count == new.index_count
        && old.topology_hash == new.topology_hash
        && old.shape_hash == new.shape_hash
}
