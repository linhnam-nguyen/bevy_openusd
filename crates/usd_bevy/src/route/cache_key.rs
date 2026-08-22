use std::hash::{BuildHasher, Hash, Hasher};

use bevy::platform::hash::FixedHasher;

use crate::read::geom::{Interpolation, MeshPrimvar, ReadMesh};

/// Deterministic key for the source data that controls `mesh_from_usd` output.
/// Transform, visibility, material binding, extent, and other projection-only
/// state are intentionally excluded so those edits can reuse the same mesh.
pub(crate) fn source_mesh_key(read: &ReadMesh) -> u64 {
    let mut hasher = FixedHasher.build_hasher();
    hash_vec3(&read.points, &mut hasher);
    read.face_vertex_counts.hash(&mut hasher);
    read.face_vertex_indices.hash(&mut hasher);
    hash_vec3_primvar(read.normals.as_ref(), &mut hasher);
    hash_vec2_primvar(read.uvs.as_ref(), &mut hasher);
    std::mem::discriminant(&read.orientation).hash(&mut hasher);
    hash_vec3_primvar(read.display_color.as_ref(), &mut hasher);
    hash_float_primvar(read.display_opacity.as_ref(), &mut hasher);
    std::mem::discriminant(&read.subdivision_scheme).hash(&mut hasher);
    hasher.finish()
}

fn hash_vec3(values: &[[f32; 3]], hasher: &mut impl Hasher) {
    values.len().hash(hasher);
    for value in values {
        for lane in value {
            lane.to_bits().hash(hasher);
        }
    }
}

fn hash_vec2(values: &[[f32; 2]], hasher: &mut impl Hasher) {
    values.len().hash(hasher);
    for value in values {
        for lane in value {
            lane.to_bits().hash(hasher);
        }
    }
}

fn hash_floats(values: &[f32], hasher: &mut impl Hasher) {
    values.len().hash(hasher);
    for value in values {
        value.to_bits().hash(hasher);
    }
}

fn hash_interpolation(value: Interpolation, hasher: &mut impl Hasher) {
    std::mem::discriminant(&value).hash(hasher);
}

fn hash_vec3_primvar(value: Option<&MeshPrimvar<[f32; 3]>>, hasher: &mut impl Hasher) {
    match value {
        Some(primvar) => {
            1_u8.hash(hasher);
            hash_vec3(&primvar.values, hasher);
            hash_interpolation(primvar.interpolation, hasher);
            primvar.indices.hash(hasher);
        }
        None => 0_u8.hash(hasher),
    }
}

fn hash_vec2_primvar(value: Option<&MeshPrimvar<[f32; 2]>>, hasher: &mut impl Hasher) {
    match value {
        Some(primvar) => {
            1_u8.hash(hasher);
            hash_vec2(&primvar.values, hasher);
            hash_interpolation(primvar.interpolation, hasher);
            primvar.indices.hash(hasher);
        }
        None => 0_u8.hash(hasher),
    }
}

fn hash_float_primvar(value: Option<&MeshPrimvar<f32>>, hasher: &mut impl Hasher) {
    match value {
        Some(primvar) => {
            1_u8.hash(hasher);
            hash_floats(&primvar.values, hasher);
            hash_interpolation(primvar.interpolation, hasher);
            primvar.indices.hash(hasher);
        }
        None => 0_u8.hash(hasher),
    }
}
