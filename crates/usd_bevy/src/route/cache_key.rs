use std::hash::{BuildHasher, Hash, Hasher};

use bevy::platform::hash::FixedHasher;

use crate::read::geom::{Interpolation, MeshPrimvar, ReadMesh};

pub(crate) const MAX_SOURCE_KEY_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MeshSourceKey(pub(crate) Box<[u8]>);

#[derive(Default)]
struct KeyBytes {
    bytes: Vec<u8>,
    oversized: bool,
}

impl Hasher for KeyBytes {
    fn finish(&self) -> u64 {
        FixedHasher.hash_one(&self.bytes)
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.oversized {
            return;
        }
        if bytes.len() > MAX_SOURCE_KEY_BYTES - self.bytes.len() {
            self.oversized = true;
            self.bytes = Vec::new();
            return;
        }
        self.bytes.extend_from_slice(bytes);
    }
}

/// Exact encoded inputs that control the current `mesh_from_usd` recipe.
/// Transform, visibility, material binding, extent, and other projection-only
/// state are intentionally excluded so those edits can reuse the same mesh.
pub(crate) fn source_mesh_key(read: &ReadMesh) -> Option<MeshSourceKey> {
    let mut hasher = KeyBytes::default();
    hasher.write(b"usd_bevy/base_mesh/v1");
    hash_vec3(&read.points, &mut hasher);
    read.face_vertex_counts.hash(&mut hasher);
    read.face_vertex_indices.hash(&mut hasher);
    hash_vec3_primvar(read.normals.as_ref(), &mut hasher);
    hash_vec2_primvar(read.uvs.as_ref(), &mut hasher);
    std::mem::discriminant(&read.orientation).hash(&mut hasher);
    hash_vec3_primvar(read.display_color.as_ref(), &mut hasher);
    hash_float_primvar(read.display_opacity.as_ref(), &mut hasher);
    std::mem::discriminant(&read.subdivision_scheme).hash(&mut hasher);
    (!hasher.oversized).then(|| MeshSourceKey(hasher.bytes.into_boxed_slice()))
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
