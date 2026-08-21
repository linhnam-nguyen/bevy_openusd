use crate::read::geom::{Interpolation, ReadMesh};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use std::time::Instant;

use super::expanded::{build_expanded, build_vertex_colors_indexed};
use super::primvar::expand_vertex_primvar;
use super::triangulation::triangulate_polygon;
use crate::route::profile::{
    GeometryInterpolation, GeometrySubdivisionClass, GeometryTopologyClass, classify_topology,
};

/// Convert a `crate::read::geom::ReadMesh` into a Bevy mesh.
///
/// Steps:
/// 1. Triangulate each face by fan (works for triangles and convex quads;
///    non-convex n-gons need an ear-clip pass we punt to M2.1).
/// 2. Expand per-vertex attributes for `faceVarying` primvars (one vertex
///    per corner) or keep indexed when interpolation is `vertex`.
/// 3. Fall back to `compute_smooth_normals` when normals aren't authored.
/// 4. Flip index winding when `orientation == LeftHanded`.
pub fn mesh_from_usd(read: &ReadMesh) -> Mesh {
    mesh_from_usd_subset(read, None)
}

/// Timings and deterministic source/output counts for one mesh conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshBuildMetrics {
    pub mesh_from_usd_ms: f64,
    pub topology_triangulation_ms: f64,
    pub primvar_expansion_ms: f64,
    pub normal_generation_ms: f64,
    pub source_points: usize,
    pub source_faces: usize,
    pub source_face_corners: usize,
    pub output_vertices: usize,
    pub output_indices: usize,
    pub output_triangles: usize,
    pub authored_normals: bool,
    pub generated_normals: bool,
    pub expanded_vertices: bool,
    pub uv_interpolation: GeometryInterpolation,
    pub indexed_primvars: usize,
    pub non_indexed_primvars: usize,
    pub expansion_forcing_primvars: usize,
    pub display_color: bool,
    pub display_opacity: bool,
    pub topology_class: GeometryTopologyClass,
    pub subdivision: GeometrySubdivisionClass,
    pub vertex_source_ratio: f64,
}

/// Profiled form of [`mesh_from_usd`].
pub fn mesh_from_usd_profiled(read: &ReadMesh) -> (Mesh, MeshBuildMetrics) {
    let mut metrics = MeshBuildMetrics::default();
    let mesh = build_mesh(read, None, Some(&mut metrics));
    (mesh, metrics)
}

/// Same as [`mesh_from_usd`] but emits only the faces in `face_subset` when
/// provided (`None` = every face). Used to split a `UsdGeom.Mesh` into one
/// Bevy mesh per `GeomSubset` so each subset can carry its own material.
pub fn mesh_from_usd_subset(read: &ReadMesh, face_subset: Option<&[i32]>) -> Mesh {
    build_mesh(read, face_subset, None)
}

fn build_mesh(
    read: &ReadMesh,
    face_subset: Option<&[i32]>,
    mut profile: Option<&mut MeshBuildMetrics>,
) -> Mesh {
    let total_start = Instant::now();
    if let Some(metrics) = profile.as_mut() {
        metrics.source_points = read.points.len();
        metrics.source_faces = read.face_vertex_counts.len();
        metrics.source_face_corners = read
            .face_vertex_counts
            .iter()
            .map(|count| (*count).max(0) as usize)
            .sum();
        metrics.uv_interpolation = read
            .uvs
            .as_ref()
            .map(|primvar| primvar.interpolation.into())
            .unwrap_or_default();
        let primvars = [
            read.normals.as_ref().map(|p| {
                (
                    !p.indices.is_empty(),
                    matches!(
                        p.interpolation,
                        Interpolation::Uniform | Interpolation::FaceVarying
                    ),
                )
            }),
            read.uvs.as_ref().map(|p| {
                (
                    !p.indices.is_empty(),
                    matches!(
                        p.interpolation,
                        Interpolation::Uniform | Interpolation::FaceVarying
                    ),
                )
            }),
            read.display_color.as_ref().map(|p| {
                (
                    !p.indices.is_empty(),
                    matches!(
                        p.interpolation,
                        Interpolation::Uniform | Interpolation::FaceVarying
                    ),
                )
            }),
            read.display_opacity.as_ref().map(|p| {
                (
                    !p.indices.is_empty(),
                    matches!(
                        p.interpolation,
                        Interpolation::Uniform | Interpolation::FaceVarying
                    ),
                )
            }),
        ]
        .into_iter()
        .flatten();
        let mut indexed_primvars = 0;
        let mut non_indexed_primvars = 0;
        let mut expansion_forcing_primvars = 0;
        for (indexed, forcing) in primvars {
            if indexed {
                indexed_primvars += 1;
            } else {
                non_indexed_primvars += 1;
            }
            if forcing {
                expansion_forcing_primvars += 1;
            }
        }
        metrics.indexed_primvars = indexed_primvars;
        metrics.non_indexed_primvars = non_indexed_primvars;
        metrics.expansion_forcing_primvars = expansion_forcing_primvars;
        metrics.authored_normals = read.normals.is_some();
        metrics.generated_normals = read.normals.is_none();
        metrics.display_color = read.display_color.is_some();
        metrics.display_opacity = read.display_opacity.is_some();
        metrics.topology_class = classify_topology(&read.face_vertex_counts);
        metrics.subdivision = read.subdivision_scheme.into();
    }
    // Face-Varying or Uniform (per-face) primvars break the indexed
    // point-sharing optimisation — vertex-indexed output can't represent
    // a per-face or per-corner value when a vertex is shared between
    // faces with different authored values. Expand to per-corner layout
    // in those cases.
    let non_indexed = |interp: Interpolation| {
        matches!(interp, Interpolation::FaceVarying | Interpolation::Uniform)
    };
    let expand = read
        .normals
        .as_ref()
        .map(|p| non_indexed(p.interpolation))
        .unwrap_or(false)
        || read
            .uvs
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false)
        || read
            .display_color
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false)
        || read
            .display_opacity
            .as_ref()
            .map(|p| non_indexed(p.interpolation))
            .unwrap_or(false);

    let (positions, normals, uvs, colors, indices) = if expand {
        build_expanded(read, face_subset, &mut profile)
    } else {
        build_indexed(read, face_subset, &mut profile)
    };

    if let Some(metrics) = profile.as_mut() {
        metrics.output_vertices = positions.len();
        metrics.output_indices = indices.len();
        metrics.output_triangles = indices.len() / 3;
        metrics.expanded_vertices = expand;
        metrics.vertex_source_ratio = if metrics.source_points == 0 {
            0.0
        } else {
            metrics.output_vertices as f64 / metrics.source_points as f64
        };
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    // USD's `primvars:st` convention puts (0,0) at the texture's
    // bottom-left corner. Bevy / glTF / wgpu use top-left, so V is
    // inverted between the two systems. Flip on the way in so the
    // authored texture lands right-side-up — without this, eyes paint
    // on tails, etc.
    let uvs: Vec<[f32; 2]> = uvs.into_iter().map(|[u, v]| [u, 1.0 - v]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(cs) = colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cs);
    }
    // Indices first so `compute_smooth_normals` has a topology to
    // average across — it requires an indexed mesh to find adjacent
    // faces.
    mesh.insert_indices(Indices::U32(indices));
    if let Some(ns) = normals {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, ns);
    } else {
        // `compute_flat_normals` replicates positions so normals are per-face
        // — correct but bloats the mesh. Smooth normals keep the original
        // topology and average adjacent face normals. For plain USD stages
        // without authored normals that's the intuitive default.
        let normal_start = Instant::now();
        mesh.compute_smooth_normals();
        if let Some(metrics) = profile.as_mut() {
            metrics.normal_generation_ms += normal_start.elapsed().as_secs_f64() * 1000.0;
        }
    }
    // MikkT vertex tangents — Bevy's PBR shader needs `ATTRIBUTE_TANGENT`
    // to evaluate normal maps correctly. Without them, normal-mapped
    // surfaces silently fall back to geometric normals and the surface
    // detail (drummer stitching, glove leather, biplane rivets) looks
    // flat. `generate_tangents` requires positions + normals + UV0 — all
    // present at this point; failures are fatal-but-rare and we just log.
    if let Err(e) = mesh.generate_tangents() {
        bevy::log::debug!("mesh: generate_tangents failed: {e}");
    }
    if let Some(metrics) = profile.as_mut() {
        metrics.mesh_from_usd_ms = total_start.elapsed().as_secs_f64() * 1000.0;
    }
    mesh
}

/// Assembled mesh attributes: `(positions, normals?, uvs, colors?, indices)`.
pub(super) type BuiltMesh = (
    Vec<[f32; 3]>,
    Option<Vec<[f32; 3]>>,
    Vec<[f32; 2]>,
    Option<Vec<[f32; 4]>>,
    Vec<u32>,
);

/// Build the common case: indexed triangle list, one vertex per USD point.
/// Uses vertex-level or constant interpolation only.
fn build_indexed(
    read: &ReadMesh,
    face_subset: Option<&[i32]>,
    profile: &mut Option<&mut MeshBuildMetrics>,
) -> BuiltMesh {
    let positions = read.points.clone();
    let primvar_start = Instant::now();

    // Normals: pick up vertex-indexed data if present; else None and let
    // `compute_smooth_normals` handle it. `Varying` is semantically
    // per-point for polygonal meshes (USD spec) so we ride the same
    // path as `Vertex` — silently dropping it would force generated
    // smooth normals over authored ones.
    let normals = read.normals.as_ref().and_then(|p| match p.interpolation {
        Interpolation::Vertex | Interpolation::Varying => {
            Some(expand_vertex_primvar(p, positions.len(), [0.0, 1.0, 0.0]))
        }
        Interpolation::Constant if !p.values.is_empty() => Some(vec![p.values[0]; positions.len()]),
        _ => None,
    });

    let uvs = read
        .uvs
        .as_ref()
        .and_then(|p| match p.interpolation {
            Interpolation::Vertex | Interpolation::Varying => {
                Some(expand_vertex_primvar(p, positions.len(), [0.0, 0.0]))
            }
            _ => None,
        })
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    let colors = build_vertex_colors_indexed(read, positions.len());
    if let Some(metrics) = profile.as_mut() {
        metrics.primvar_expansion_ms += primvar_start.elapsed().as_secs_f64() * 1000.0;
    }

    let triangulation_start = Instant::now();
    let indices = triangulate_polygon(
        &positions,
        &read.face_vertex_counts,
        &read.face_vertex_indices,
        read.orientation,
        face_subset,
    );
    if let Some(metrics) = profile.as_mut() {
        metrics.topology_triangulation_ms += triangulation_start.elapsed().as_secs_f64() * 1000.0;
    }
    (positions, normals, uvs, colors, indices)
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
