use crate::read::geom::Orientation;
use bevy::math::Vec3;

/// Triangulate each face into a triangle list. Smart enough for the three
/// cases real USD assets throw at us:
///
/// - **n = 3**: emit as-is.
/// - **n = 4** (the dominant case in production assets): pick the *shorter*
///   diagonal. Non-planar quads — almost universal in subdivided cages and
///   imported FBX — produce a visible crease along whichever diagonal a fan
///   triangulator picks. Choosing the diagonal that minimises the triangle
///   pair's perimeter aligns the crease with the surface curvature, which
///   is what every offline renderer (and Maya/Blender's default) does.
/// - **n ≥ 5**: ear-clip. Fan triangulation of a concave n-gon emits
///   triangles *outside* the polygon (showing through to the back) and
///   misses parts inside it — which is exactly the "spiky / missing
///   triangles" symptom on production-asset n-gons. Ear clipping handles
///   concave faces correctly. We compute the polygon normal via Newell's
///   method (works on non-planar polygons too) and pick ears in 2D after
///   projecting onto the plane perpendicular to that normal.
///
/// Falls back to fan triangulation if the polygon is degenerate (all colinear
/// points) — emitting *something* matches USD's permissive behaviour.
///
/// `LeftHanded` orientation flips the winding so Bevy's default back-face
/// culling shows the right side.
///
/// `face_subset = Some(&[face_ix])` emits only the listed faces — used by
/// the GeomSubset per-material split.
pub(super) fn triangulate_polygon(
    positions: &[[f32; 3]],
    counts: &[i32],
    indices: &[i32],
    orientation: Orientation,
    face_subset: Option<&[i32]>,
) -> Vec<u32> {
    // No vertices → no triangles. Emitting indices into an empty buffer would
    // later panic Bevy's normal/tangent generation.
    if positions.is_empty() {
        return Vec::new();
    }
    // Precompute each face's starting corner so a subset by face index
    // jumps straight to the right slice without rewalking the counts. Negative
    // counts (malformed USD) contribute zero rather than wrapping to a huge
    // `usize` that would overflow the running sum.
    let mut face_starts = Vec::with_capacity(counts.len());
    let mut running = 0usize;
    for c in counts {
        face_starts.push(running);
        running += (*c).max(0) as usize;
    }

    let face_iter: Box<dyn Iterator<Item = usize>> = match face_subset {
        None => Box::new(0..counts.len()),
        Some(sub) => Box::new(
            sub.iter()
                .map(|i| *i as usize)
                .filter(|i| *i < counts.len()),
        ),
    };

    let mut out = Vec::new();
    let emit = |out: &mut Vec<u32>, a: u32, b: u32, c: u32| match orientation {
        Orientation::RightHanded => out.extend_from_slice(&[a, b, c]),
        Orientation::LeftHanded => out.extend_from_slice(&[a, c, b]),
    };
    let nv = positions.len();
    // Read a corner's vertex index, tolerating an index array shorter than the
    // counts imply (malformed USD) — a missing corner reads as index 0.
    let idx_at = |c: usize| -> i32 { indices.get(c).copied().unwrap_or(0) };
    // Clamp a raw (possibly out-of-range or negative) point index so an emitted
    // mesh index never points past the vertex buffer (which the GPU would read
    // out of bounds).
    let clamp_v = |i: i32| -> u32 {
        if nv == 0 {
            0
        } else {
            (i.max(0) as usize).min(nv - 1) as u32
        }
    };
    let pos_of = |idx: i32| -> Vec3 {
        match positions.get(idx.max(0) as usize) {
            Some(p) => Vec3::new(p[0], p[1], p[2]),
            None => Vec3::ZERO,
        }
    };

    for face_ix in face_iter {
        let face_start = face_starts[face_ix];
        let n = counts[face_ix].max(0) as usize;
        if n < 3 {
            continue;
        }
        if n == 3 {
            let a = clamp_v(idx_at(face_start));
            let b = clamp_v(idx_at(face_start + 1));
            let c = clamp_v(idx_at(face_start + 2));
            emit(&mut out, a, b, c);
            continue;
        }
        if n == 4 {
            let i0 = idx_at(face_start);
            let i1 = idx_at(face_start + 1);
            let i2 = idx_at(face_start + 2);
            let i3 = idx_at(face_start + 3);
            // Pick the shorter diagonal: 0–2 vs 1–3.
            let p0 = pos_of(i0);
            let p1 = pos_of(i1);
            let p2 = pos_of(i2);
            let p3 = pos_of(i3);
            let d02 = (p2 - p0).length_squared();
            let d13 = (p3 - p1).length_squared();
            if d02 <= d13 {
                emit(&mut out, clamp_v(i0), clamp_v(i1), clamp_v(i2));
                emit(&mut out, clamp_v(i0), clamp_v(i2), clamp_v(i3));
            } else {
                emit(&mut out, clamp_v(i1), clamp_v(i2), clamp_v(i3));
                emit(&mut out, clamp_v(i1), clamp_v(i3), clamp_v(i0));
            }
            continue;
        }
        // n >= 5: ear clip. Clamp the slice end so a counts/indices mismatch
        // can't panic; skip the face if fewer than a triangle survives. Corner
        // indices are pre-clamped so ear-clip's emitted mesh indices stay valid.
        let end = (face_start + n).min(indices.len());
        if end.saturating_sub(face_start) < 3 {
            continue;
        }
        let face_indices: Vec<i32> = indices[face_start..end]
            .iter()
            .map(|i| clamp_v(*i) as i32)
            .collect();
        let face_positions: Vec<Vec3> = face_indices.iter().map(|i| pos_of(*i)).collect();
        ear_clip_into(&face_positions, &face_indices, &mut out, orientation);
    }
    out
}

/// Ear-clip a polygon (n ≥ 4 in practice) into triangles, appending into
/// `out`. Robust against concave polygons; for non-planar polygons we
/// project onto the plane perpendicular to the Newell normal so the 2D
/// containment test is meaningful.
///
/// Falls back to fan triangulation if no ears can be found (e.g. fully
/// degenerate / self-intersecting input). That matches USD's "translate
/// what you can, drop nothing" expectation.
fn ear_clip_into(
    positions: &[Vec3],
    indices: &[i32],
    out: &mut Vec<u32>,
    orientation: Orientation,
) {
    let n = positions.len();
    let emit = |out: &mut Vec<u32>, a: u32, b: u32, c: u32| match orientation {
        Orientation::RightHanded => out.extend_from_slice(&[a, b, c]),
        Orientation::LeftHanded => out.extend_from_slice(&[a, c, b]),
    };

    // Newell's method: robust normal even for non-planar polygons. Sums
    // per-edge cross-products of the projected components.
    let mut normal = Vec3::ZERO;
    for i in 0..n {
        let cur = positions[i];
        let nxt = positions[(i + 1) % n];
        normal.x += (cur.y - nxt.y) * (cur.z + nxt.z);
        normal.y += (cur.z - nxt.z) * (cur.x + nxt.x);
        normal.z += (cur.x - nxt.x) * (cur.y + nxt.y);
    }
    if normal.length_squared() < 1e-20 {
        // Degenerate polygon — fall back to fan.
        for k in 1..(n - 1) {
            emit(
                out,
                indices[0] as u32,
                indices[k] as u32,
                indices[k + 1] as u32,
            );
        }
        return;
    }
    let normal = normal.normalize();

    // Build orthonormal basis (u, v) on the polygon plane to project into
    // 2D. Pick the smallest absolute component of the normal as the
    // helper axis to avoid degeneracy.
    let helper = if normal.x.abs() < normal.y.abs() && normal.x.abs() < normal.z.abs() {
        Vec3::X
    } else if normal.y.abs() < normal.z.abs() {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let u = normal.cross(helper).normalize();
    let v = normal.cross(u);
    let project = |p: Vec3| -> [f32; 2] { [p.dot(u), p.dot(v)] };

    let pts2: Vec<[f32; 2]> = positions.iter().map(|p| project(*p)).collect();

    // Determine polygon winding in 2D. Signed area > 0 → CCW.
    let mut signed_area = 0.0f32;
    for i in 0..n {
        let a = pts2[i];
        let b = pts2[(i + 1) % n];
        signed_area += a[0] * b[1] - b[0] * a[1];
    }
    let ccw = signed_area > 0.0;

    // Active vertex list (linked-list-style via Vec).
    let mut remaining: Vec<usize> = (0..n).collect();
    // Worst-case ear clipping is O(n²) but n is tiny (≤ ~10 in practice).
    let max_iters = n * n + 8;
    let mut iters = 0;
    while remaining.len() > 3 && iters < max_iters {
        iters += 1;
        let m = remaining.len();
        let mut clipped = false;
        for i in 0..m {
            let i_prev = remaining[(i + m - 1) % m];
            let i_cur = remaining[i];
            let i_next = remaining[(i + 1) % m];
            let a = pts2[i_prev];
            let b = pts2[i_cur];
            let c = pts2[i_next];
            // Convex test in chosen winding.
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            let convex = if ccw { cross > 0.0 } else { cross < 0.0 };
            if !convex {
                continue;
            }
            // Ear test: no other remaining vertex inside triangle (a,b,c).
            let mut contains_other = false;
            for &idx in &remaining {
                if idx == i_prev || idx == i_cur || idx == i_next {
                    continue;
                }
                let p = pts2[idx];
                if point_in_triangle_2d(p, a, b, c) {
                    contains_other = true;
                    break;
                }
            }
            if contains_other {
                continue;
            }
            // Emit and clip.
            emit(
                out,
                indices[i_prev] as u32,
                indices[i_cur] as u32,
                indices[i_next] as u32,
            );
            remaining.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // No ear found — bail out and fan-triangulate the rest.
            break;
        }
    }
    if remaining.len() == 3 {
        emit(
            out,
            indices[remaining[0]] as u32,
            indices[remaining[1]] as u32,
            indices[remaining[2]] as u32,
        );
    } else if remaining.len() > 3 {
        // Fallback: fan over what's left.
        let r0 = remaining[0];
        for k in 1..(remaining.len() - 1) {
            emit(
                out,
                indices[r0] as u32,
                indices[remaining[k]] as u32,
                indices[remaining[k + 1]] as u32,
            );
        }
    }
}

/// Standard barycentric inside-triangle test. Includes points exactly on
/// edges (we still emit ears even if a vertex sits on an edge — the
/// alternative is endless retries on collinear data).
fn point_in_triangle_2d(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d_x = p[0] - c[0];
    let d_y = p[1] - c[1];
    let denom = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if denom.abs() < 1e-20 {
        return false;
    }
    let s = ((b[1] - c[1]) * d_x + (c[0] - b[0]) * d_y) / denom;
    let t = ((c[1] - a[1]) * d_x + (a[0] - c[0]) * d_y) / denom;
    s > 0.0 && t > 0.0 && (s + t) < 1.0
}
