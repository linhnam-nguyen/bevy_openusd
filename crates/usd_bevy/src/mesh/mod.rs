//! UsdGeom → `bevy::render::mesh::Mesh`.
//!
//! Two kinds of input:
//! - Full meshes (`UsdGeom.Mesh`) — converts points / face indices / normals
//!   / uvs, fan-triangulates faces > 3 verts, expands `faceVarying` primvars.
//! - Primitive shapes (`Cube`, `Sphere`, `Cylinder`, `Capsule`) — delegate
//!   to Bevy's built-in `Meshable` primitives with the right dimensions.
//!
//! Orientation (`"leftHanded"` flips winding) and missing-normal fallback
//! (`compute_smooth_normals`) are handled here.

mod builder;
mod expanded;
mod normals;
mod primitives;
mod primvar;
mod triangulation;

pub(crate) use builder::output_point_indices;
pub use builder::{MeshBuildMetrics, mesh_from_usd, mesh_from_usd_profiled, mesh_from_usd_subset};
pub use primitives::{
    mesh_capsule, mesh_cube, mesh_cylinder, mesh_plane, mesh_sphere, rotate_mesh,
};

use crate::read::geom::ReadMesh;
use bevy::mesh::VertexAttributeValues;

/// The native four-influence representation used by Bevy's skinning shader.
#[derive(Clone, Debug)]
pub(crate) struct SkinAttrs {
    pub(crate) indices: Vec<[u16; 4]>,
    pub(crate) weights: Vec<[f32; 4]>,
}

/// Convert a composed UsdSkel binding to Bevy's four-wide vertex attributes.
pub(crate) fn skin_attrs_from_binding(
    binding: &openusd::schemas::skel::SkelBindingAPI,
    vertex_count: usize,
    joint_count: usize,
) -> anyhow::Result<SkinAttrs> {
    let indices = binding.joint_indices()?;
    let weights = binding.joint_weights()?;
    let influences = binding.elements_per_element()?.max(1) as usize;
    let constant = matches!(
        binding.interpolation()?,
        openusd::schemas::skel::InfluenceInterpolation::Constant
    );
    let mut output_indices = vec![[0; 4]; vertex_count];
    let mut output_weights = vec![[0.0; 4]; vertex_count];
    for vertex in 0..vertex_count {
        let base = if constant { 0 } else { vertex * influences };
        let mut candidates = (0..influences)
            .filter_map(|slot| {
                let joint = indices.get(base + slot).copied()?.max(0) as usize;
                let weight = weights
                    .get(base + slot)
                    .copied()
                    .unwrap_or_default()
                    .max(0.0);
                (joint < joint_count).then_some((joint as u16, weight))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut sum = 0.0;
        for (slot, (joint, weight)) in candidates.into_iter().take(4).enumerate() {
            output_indices[vertex][slot] = joint;
            output_weights[vertex][slot] = weight;
            sum += weight;
        }
        if sum > 0.0 {
            for weight in &mut output_weights[vertex] {
                *weight /= sum;
            }
        } else {
            output_weights[vertex][0] = 1.0;
        }
    }
    Ok(SkinAttrs {
        indices: output_indices,
        weights: output_weights,
    })
}

/// Build the static mesh plus native Bevy skinning attributes. Playback never
/// calls this function; it updates only the bound joint entities.
pub(crate) fn mesh_from_usd_with_skin(read: &ReadMesh, skin: &SkinAttrs) -> bevy::mesh::Mesh {
    let mut mesh = mesh_from_usd(read);
    let points = output_point_indices(read);
    let indices = points
        .iter()
        .map(|&point| skin.indices.get(point).copied().unwrap_or([0; 4]))
        .collect::<Vec<_>>();
    let weights = points
        .iter()
        .map(|&point| {
            skin.weights
                .get(point)
                .copied()
                .unwrap_or([1.0, 0.0, 0.0, 0.0])
        })
        .collect::<Vec<_>>();
    mesh.insert_attribute(
        bevy::mesh::Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(indices),
    );
    mesh.insert_attribute(
        bevy::mesh::Mesh::ATTRIBUTE_JOINT_WEIGHT,
        VertexAttributeValues::Float32x4(weights),
    );
    mesh
}
