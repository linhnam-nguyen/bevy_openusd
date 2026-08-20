//! Dimension-level classification for entities matched by identity.

use usd_model::{ChangeFlags, EntitySnapshot, GeometrySignature};

/// Classify all semantic dimensions that differ between two existing entities.
pub(crate) fn classify_existing(old: &EntitySnapshot, new: &EntitySnapshot) -> ChangeFlags {
    let mut flags = ChangeFlags::empty();

    if old.transform != new.transform {
        flags |= ChangeFlags::TRANSFORM;
    }
    if geometry_changed(old.geometry.as_ref(), new.geometry.as_ref()) {
        flags |= ChangeFlags::GEOMETRY;
    }
    if old.metadata_hash != new.metadata_hash {
        flags |= ChangeFlags::METADATA;
    }
    if old.prim_path != new.prim_path {
        flags |= ChangeFlags::PATH;
    }

    flags
}

fn geometry_changed(old: Option<&GeometrySignature>, new: Option<&GeometrySignature>) -> bool {
    match (old, new) {
        (None, None) => false,
        (None, Some(_)) | (Some(_), None) => true,
        (Some(old), Some(new)) => {
            // Bounds are retained as raw floats for display and are derived
            // from the points. Quantized counts/centroid and content hashes
            // carry the canonical semantic geometry identity, so epsilon
            // noise in raw bounds must not become a geometry change.
            old.vertex_count != new.vertex_count
                || old.index_count != new.index_count
                || old.local_centroid != new.local_centroid
                || old.topology_hash != new.topology_hash
                || old.shape_hash != new.shape_hash
                || old.render_blob != new.render_blob
        }
    }
}
