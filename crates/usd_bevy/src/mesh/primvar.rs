use crate::read::geom::{Interpolation, MeshPrimvar, ReadMesh};

pub(super) fn corner_normal(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 3] {
    let p = read.normals.as_ref().unwrap();
    sample_primvar_3(p, face, corner, point, [0.0, 1.0, 0.0])
}

pub(super) fn corner_color(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 4] {
    let rgb_fallback = [1.0_f32, 1.0, 1.0];
    let rgb = read
        .display_color
        .as_ref()
        .map(|dc| sample_primvar_3(dc, face, corner, point, rgb_fallback))
        .unwrap_or(rgb_fallback);
    let a = read
        .display_opacity
        .as_ref()
        .map(|dop| sample_primvar_1(dop, face, corner, point, 1.0))
        .unwrap_or(1.0);
    [rgb[0], rgb[1], rgb[2], a]
}

/// Sample a vec3 primvar at a specific corner. Single-value primvars
/// broadcast regardless of declared interpolation — Pixar's
/// Kitchen_set authors `primvars:displayColor = [(0.5, 0.5, 0.4)]`
/// without an `interpolation` token; the schema reader's default of
/// `Vertex` then fails to expand the 1-element array to vertex_count
/// and falls back to white.
pub(super) fn sample_primvar_3(
    p: &MeshPrimvar<[f32; 3]>,
    face: usize,
    corner: usize,
    point: usize,
    fallback: [f32; 3],
) -> [f32; 3] {
    if p.values.len() == 1 {
        return p.values[0];
    }
    let lookup = |slot: usize| -> [f32; 3] {
        let ix = if !p.indices.is_empty() {
            *p.indices.get(slot).unwrap_or(&0) as usize
        } else {
            slot
        };
        p.values.get(ix).copied().unwrap_or(fallback)
    };
    match p.interpolation {
        Interpolation::Constant => p.values.first().copied().unwrap_or(fallback),
        Interpolation::Uniform => lookup(face),
        Interpolation::Vertex | Interpolation::Varying => lookup(point),
        Interpolation::FaceVarying => lookup(corner),
    }
}

pub(super) fn sample_primvar_1(
    p: &MeshPrimvar<f32>,
    face: usize,
    corner: usize,
    point: usize,
    fallback: f32,
) -> f32 {
    if p.values.len() == 1 {
        return p.values[0];
    }
    let lookup = |slot: usize| -> f32 {
        let ix = if !p.indices.is_empty() {
            *p.indices.get(slot).unwrap_or(&0) as usize
        } else {
            slot
        };
        p.values.get(ix).copied().unwrap_or(fallback)
    };
    match p.interpolation {
        Interpolation::Constant => p.values.first().copied().unwrap_or(fallback),
        Interpolation::Uniform => lookup(face),
        Interpolation::Vertex | Interpolation::Varying => lookup(point),
        Interpolation::FaceVarying => lookup(corner),
    }
}

pub(super) fn corner_uv(read: &ReadMesh, face: usize, corner: usize, point: usize) -> [f32; 2] {
    let p = read.uvs.as_ref().unwrap();
    let fallback = [0.0, 0.0];
    if p.values.len() == 1 {
        return p.values[0];
    }
    match p.interpolation {
        Interpolation::Constant => p.values.first().copied().unwrap_or(fallback),
        Interpolation::Uniform => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(face).unwrap_or(&0) as usize
            } else {
                face
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
        Interpolation::Vertex | Interpolation::Varying => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(point).unwrap_or(&0) as usize
            } else {
                point
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
        Interpolation::FaceVarying => {
            let ix = if !p.indices.is_empty() {
                *p.indices.get(corner).unwrap_or(&0) as usize
            } else {
                corner
            };
            p.values.get(ix).copied().unwrap_or(fallback)
        }
    }
}

pub(super) fn expand_vertex_primvar<T: Copy>(
    primvar: &MeshPrimvar<T>,
    expected_len: usize,
    fallback: T,
) -> Vec<T> {
    // Always emit exactly `expected_len` entries — Bevy 0.18 silently
    // drops the mesh if attribute lengths don't match
    // `ATTRIBUTE_POSITION`. Pad with `fallback` if the authored data
    // is short, truncate if it's long.
    let mut out = vec![fallback; expected_len];
    if primvar.indices.is_empty() {
        for (i, v) in primvar.values.iter().take(expected_len).enumerate() {
            out[i] = *v;
        }
    } else {
        for (i, ix) in primvar.indices.iter().take(expected_len).enumerate() {
            if let Some(v) = primvar.values.get(*ix as usize) {
                out[i] = *v;
            }
        }
    }
    out
}
