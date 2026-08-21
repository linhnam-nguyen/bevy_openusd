use crate::read::geom::{Interpolation, ReadMesh};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use std::time::Instant;

use super::normals::compute_point_smooth_normals;
use super::primvar::{corner_color, corner_normal, corner_uv, expand_vertex_primvar};
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
    pub expanded_primvars: usize,
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
        (**metrics).source_points = read.points.len();
        (**metrics).source_faces = read.face_vertex_counts.len();
        (**metrics).source_face_corners = read
            .face_vertex_counts
            .iter()
            .map(|count| (*count).max(0) as usize)
            .sum();
        (**metrics).uv_interpolation = read
            .uvs
            .as_ref()
            .map(|primvar| primvar.interpolation.into())
            .unwrap_or_default();
        (**metrics).indexed_primvars = [
            read.normals.as_ref().map(|p| p.interpolation),
            read.uvs.as_ref().map(|p| p.interpolation),
            read.display_color.as_ref().map(|p| p.interpolation),
            read.display_opacity.as_ref().map(|p| p.interpolation),
        ]
        .into_iter()
        .flatten()
        .filter(|interpolation| {
            matches!(
                interpolation,
                Interpolation::Constant | Interpolation::Varying | Interpolation::Vertex
            )
        })
        .count();
        (**metrics).expanded_primvars = [
            read.normals.as_ref().map(|p| p.interpolation),
            read.uvs.as_ref().map(|p| p.interpolation),
            read.display_color.as_ref().map(|p| p.interpolation),
            read.display_opacity.as_ref().map(|p| p.interpolation),
        ]
        .into_iter()
        .flatten()
        .filter(|interpolation| {
            matches!(
                interpolation,
                Interpolation::Uniform | Interpolation::FaceVarying
            )
        })
        .count();
        (**metrics).display_color = read.display_color.is_some();
        (**metrics).display_opacity = read.display_opacity.is_some();
        (**metrics).topology_class = classify_topology(&read.face_vertex_counts);
        (**metrics).subdivision = read.subdivision_scheme.into();
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
        metrics.authored_normals = normals.is_some();
        metrics.generated_normals = normals.is_none();
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
type BuiltMesh = (
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
        (**metrics).primvar_expansion_ms += primvar_start.elapsed().as_secs_f64() * 1000.0;
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
        (**metrics).topology_triangulation_ms +=
            triangulation_start.elapsed().as_secs_f64() * 1000.0;
    }
    (positions, normals, uvs, colors, indices)
}

/// For indexed output, displayColor / displayOpacity only contribute when
/// they're vertex- or constant-interpolated (faceVarying/uniform force the
/// expanded path). Returns `None` when there's nothing to emit so the
/// caller can skip writing the attribute at all.
fn build_vertex_colors_indexed(read: &ReadMesh, vertex_count: usize) -> Option<Vec<[f32; 4]>> {
    if read.display_color.is_none() && read.display_opacity.is_none() {
        return None;
    }
    let mut colors = vec![[1.0f32, 1.0, 1.0, 1.0]; vertex_count];
    if let Some(dc) = read.display_color.as_ref() {
        let rgbs = match dc.interpolation {
            Interpolation::Constant if !dc.values.is_empty() => {
                vec![dc.values[0]; vertex_count]
            }
            // Single-value primvar — broadcast regardless of which
            // interpolation token was authored. Pixar's Kitchen_set
            // authors `primvars:displayColor = [(0.5, 0.5, 0.4)]`
            // with no `interpolation` token; the schema reader's
            // default of `Vertex` then fails to expand a 1-element
            // array to vertex_count and falls through to white.
            _ if dc.values.len() == 1 => vec![dc.values[0]; vertex_count],
            // `Varying` is semantically per-vertex for polygonal meshes,
            // so it rides the same indexed path as `Vertex`.
            Interpolation::Vertex | Interpolation::Varying => {
                expand_vertex_primvar(dc, vertex_count, [1.0, 1.0, 1.0])
            }
            _ => vec![[1.0, 1.0, 1.0]; vertex_count],
        };
        for (i, rgb) in rgbs.iter().enumerate() {
            colors[i][0] = rgb[0];
            colors[i][1] = rgb[1];
            colors[i][2] = rgb[2];
        }
    }
    if let Some(dop) = read.display_opacity.as_ref() {
        let alphas = match dop.interpolation {
            Interpolation::Constant if !dop.values.is_empty() => {
                vec![dop.values[0]; vertex_count]
            }
            // Single-value primvar — broadcast regardless of declared
            // interpolation (see `display_color` arm above for the
            // Pixar Kitchen_set rationale).
            _ if dop.values.len() == 1 => vec![dop.values[0]; vertex_count],
            Interpolation::Vertex | Interpolation::Varying => {
                expand_vertex_primvar(dop, vertex_count, 1.0)
            }
            _ => vec![1.0; vertex_count],
        };
        for (i, a) in alphas.iter().enumerate() {
            colors[i][3] = *a;
        }
    }
    Some(colors)
}

/// Build the fully-expanded form: one vertex per face corner so `faceVarying`
/// primvars (cube uvs, seams) can be represented.
fn build_expanded(
    read: &ReadMesh,
    face_subset: Option<&[i32]>,
    profile: &mut Option<&mut MeshBuildMetrics>,
) -> BuiltMesh {
    let corner_count: usize = read
        .face_vertex_counts
        .iter()
        .map(|c| (*c).max(0) as usize)
        .sum();
    let mut positions = Vec::with_capacity(corner_count);
    let mut normals_out: Vec<[f32; 3]> = Vec::with_capacity(corner_count);
    let mut uvs_out: Vec<[f32; 2]> = Vec::with_capacity(corner_count);
    let mut colors_out: Vec<[f32; 4]> = Vec::with_capacity(corner_count);

    let want_normals = read.normals.is_some();
    let want_uvs = read.uvs.is_some();
    let want_colors = read.display_color.is_some() || read.display_opacity.is_some();

    // When normals aren't authored, compute them on the *unexpanded*
    // point-indexed mesh so vertices shared between faces produce a
    // smoothed (averaged) normal. If we let `Mesh::compute_smooth_normals`
    // run after expansion, every corner is its own vertex (because some
    // other primvar — usually FaceVarying UVs for texture seams — forced
    // expansion), so "smooth" normals collapse to face normals and you
    // see every polygon. Compute once, then index per corner.
    let normal_start = Instant::now();
    let smooth_per_point: Option<Vec<[f32; 3]>> =
        (!want_normals).then(|| compute_point_smooth_normals(read));
    if let Some(metrics) = profile.as_mut() {
        (**metrics).normal_generation_ms += normal_start.elapsed().as_secs_f64() * 1000.0;
    }

    let primvar_start = Instant::now();
    let mut corner_ix: usize = 0;
    for (face_ix, face_verts) in read.face_vertex_counts.iter().enumerate() {
        for k in 0..((*face_verts).max(0) as usize) {
            // Tolerate malformed indices: a missing corner reads as 0, and an
            // index past the point buffer clamps to the last point (never OOB).
            let raw = read
                .face_vertex_indices
                .get(corner_ix + k)
                .copied()
                .unwrap_or(0);
            let point_ix = (raw.max(0) as usize).min(read.points.len().saturating_sub(1));
            positions.push(
                read.points
                    .get(point_ix)
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0]),
            );
            if want_normals {
                normals_out.push(corner_normal(read, face_ix, corner_ix + k, point_ix));
            } else if let Some(ref ns) = smooth_per_point {
                normals_out.push(*ns.get(point_ix).unwrap_or(&[0.0, 1.0, 0.0]));
            }
            if want_uvs {
                uvs_out.push(corner_uv(read, face_ix, corner_ix + k, point_ix));
            } else {
                // Pad UVs so `ATTRIBUTE_UV_0` always has the same
                // length as `ATTRIBUTE_POSITION` — mismatched lengths
                // make Bevy silently drop the mesh.
                uvs_out.push([0.0, 0.0]);
            }
            if want_colors {
                colors_out.push(corner_color(read, face_ix, corner_ix + k, point_ix));
            }
        }
        corner_ix += (*face_verts).max(0) as usize;
    }
    if let Some(metrics) = profile.as_mut() {
        (**metrics).primvar_expansion_ms += primvar_start.elapsed().as_secs_f64() * 1000.0;
    }

    // After expansion, indices become sequential 0..N per face, then
    // fan-triangulated. Re-derive a pseudo `faceVertexIndices` of the form
    // [0,1,2,3, 4,5,6, …] so `triangulate_fan` can do its job.
    let mut sequential = Vec::with_capacity(corner_count);
    let mut running = 0u32;
    for face_verts in &read.face_vertex_counts {
        for _ in 0..*face_verts {
            sequential.push(running as i32);
            running += 1;
        }
    }
    let triangulation_start = Instant::now();
    let indices = triangulate_polygon(
        &positions,
        &read.face_vertex_counts,
        &sequential,
        read.orientation,
        face_subset,
    );
    if let Some(metrics) = profile.as_mut() {
        (**metrics).topology_triangulation_ms +=
            triangulation_start.elapsed().as_secs_f64() * 1000.0;
    }

    // We emit normals whenever they were authored OR we synthesised
    // them from the point-smooth pass. The latter is the difference
    // between "smooth like Hydra" and "every face is visible".
    let emit_normals = want_normals || smooth_per_point.is_some();
    (
        positions,
        emit_normals.then_some(normals_out),
        uvs_out,
        want_colors.then_some(colors_out),
        indices,
    )
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
