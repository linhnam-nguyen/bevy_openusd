use crate::read::geom::{Orientation, ReadMesh};
use bevy::math::Vec3;

/// Per-point area-weighted smooth normals on the *unexpanded* mesh.
/// USD's faceVertexIndices is a flat per-corner list; we accumulate each
/// face's plane normal (scaled by 2× area) into all its corner points.
/// Ear-/fan-decompose larger faces just like the renderer does — using
/// (a, b, c) for k=1..n-1 keeps the contribution proportional to face
/// area for convex polygons and is good enough for concave ones since
/// a missing area cancels symmetrically.
///
/// Returns `read.points.len()` normals, normalised. Vertices unreferenced
/// by any face fall back to (0,1,0).
pub(super) fn compute_point_smooth_normals(read: &ReadMesh) -> Vec<[f32; 3]> {
    let mut accum = vec![Vec3::ZERO; read.points.len()];
    // Tolerate a short/oversized `faceVertexIndices` (malformed USD): a missing
    // corner reads as index 0, and any index past the point buffer is skipped
    // rather than panicking.
    let fvi = |c: usize| -> usize {
        read.face_vertex_indices.get(c).copied().unwrap_or(0).max(0) as usize
    };
    let mut corner_ix = 0usize;
    for face_verts in &read.face_vertex_counts {
        let n = (*face_verts).max(0) as usize;
        if n >= 3 {
            let i0 = fvi(corner_ix);
            if let Some(p0a) = read.points.get(i0) {
                let p0 = Vec3::from_array(*p0a);
                for k in 1..(n - 1) {
                    let i1 = fvi(corner_ix + k);
                    let i2 = fvi(corner_ix + k + 1);
                    if let (Some(p1a), Some(p2a)) = (read.points.get(i1), read.points.get(i2)) {
                        let p1 = Vec3::from_array(*p1a);
                        let p2 = Vec3::from_array(*p2a);
                        let face_n = match read.orientation {
                            Orientation::RightHanded => (p1 - p0).cross(p2 - p0),
                            Orientation::LeftHanded => (p2 - p0).cross(p1 - p0),
                        };
                        if let Some(a) = accum.get_mut(i0) {
                            *a += face_n;
                        }
                        if let Some(a) = accum.get_mut(i1) {
                            *a += face_n;
                        }
                        if let Some(a) = accum.get_mut(i2) {
                            *a += face_n;
                        }
                    }
                }
            }
        }
        corner_ix += n;
    }
    accum
        .into_iter()
        .map(|v| {
            if v.length_squared() > 1e-20 {
                let n = v.normalize();
                [n.x, n.y, n.z]
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}
