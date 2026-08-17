//! Quantized transform and geometry signatures.

use crate::hash::HashDigest;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct QuantizedPoint3(pub [i64; 3]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformSignature {
    pub translation_mm: [i64; 3],
    pub rotation_quantized: [i32; 4],
    pub scale_quantized: [i32; 3],
    pub hash: HashDigest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometrySignature {
    pub vertex_count: u32,
    pub index_count: u32,
    pub local_bounds: Bounds3,
    pub local_centroid: QuantizedPoint3,
    pub topology_hash: HashDigest,
    pub shape_hash: HashDigest,
    pub render_blob: Option<BlobId>,
}

/// Content-addressed identifier for a derived render payload.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobId(pub String);
