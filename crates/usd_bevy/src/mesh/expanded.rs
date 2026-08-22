use std::time::Instant;

use crate::read::geom::{Interpolation, ReadMesh};

use super::builder::{BuiltMesh, MeshBuildMetrics};
use super::normals::compute_point_smooth_normals;
use super::primvar::{corner_color, corner_normal, corner_uv, expand_vertex_primvar};
use super::triangulation::triangulate_polygon;

/// For indexed output, displayColor / displayOpacity only contribute when
/// they're vertex- or constant-interpolated (faceVarying/uniform force the
/// expanded path). Returns `None` when there's nothing to emit.
pub(super) fn build_vertex_colors_indexed(
    read: &ReadMesh,
    vertex_count: usize,
) -> Option<Vec<[f32; 4]>> {
    if read.display_color.is_none() && read.display_opacity.is_none() {
        return None;
    }
    let mut colors = vec![[1.0f32, 1.0, 1.0, 1.0]; vertex_count];
    if let Some(dc) = read.display_color.as_ref() {
        let rgbs = match dc.interpolation {
            Interpolation::Constant if !dc.values.is_empty() => {
                vec![dc.values[0]; vertex_count]
            }
            // Single-value primvar — broadcast regardless of interpolation.
            _ if dc.values.len() == 1 => vec![dc.values[0]; vertex_count],
            // `Varying` is semantically per-vertex for polygonal meshes.
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
            _ if dop.values.len() == 1 => vec![dop.values[0]; vertex_count],
            Interpolation::Vertex | Interpolation::Varying => {
                expand_vertex_primvar(dop, vertex_count, 1.0)
            }
            _ => vec![1.0; vertex_count],
        };
        for (i, alpha) in alphas.iter().enumerate() {
            colors[i][3] = *alpha;
        }
    }
    Some(colors)
}

/// Build the fully-expanded form: one vertex per face corner so
/// faceVarying primvars (cube UVs, seams) can be represented.
pub(super) fn build_expanded(
    read: &ReadMesh,
    face_subset: Option<&[i32]>,
    profile: &mut Option<&mut MeshBuildMetrics>,
) -> BuiltMesh {
    let corner_count: usize = read
        .face_vertex_counts
        .iter()
        .map(|count| (*count).max(0) as usize)
        .sum();
    let mut positions = Vec::with_capacity(corner_count);
    let mut normals_out: Vec<[f32; 3]> = Vec::with_capacity(corner_count);
    let mut uvs_out: Vec<[f32; 2]> = Vec::with_capacity(corner_count);
    let mut colors_out: Vec<[f32; 4]> = Vec::with_capacity(corner_count);

    let want_normals = read.normals.is_some();
    let want_uvs = read.uvs.is_some();
    let want_colors = read.display_color.is_some() || read.display_opacity.is_some();

    // Compute missing normals on unexpanded points so shared points retain
    // smooth normals even when another primvar forces corner expansion.
    let normal_start = Instant::now();
    let smooth_per_point: Option<Vec<[f32; 3]>> =
        (!want_normals).then(|| compute_point_smooth_normals(read));
    if let Some(metrics) = profile.as_mut() {
        metrics.normal_generation_ms += normal_start.elapsed().as_secs_f64() * 1000.0;
    }

    let primvar_start = Instant::now();
    let mut corner_ix = 0usize;
    for (face_ix, face_verts) in read.face_vertex_counts.iter().enumerate() {
        for k in 0..((*face_verts).max(0) as usize) {
            // Tolerate malformed indices without indexing outside the point buffer.
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
            } else if let Some(ref normals) = smooth_per_point {
                normals_out.push(*normals.get(point_ix).unwrap_or(&[0.0, 1.0, 0.0]));
            }
            if want_uvs {
                uvs_out.push(corner_uv(read, face_ix, corner_ix + k, point_ix));
            } else {
                uvs_out.push([0.0, 0.0]);
            }
            if want_colors {
                colors_out.push(corner_color(read, face_ix, corner_ix + k, point_ix));
            }
        }
        corner_ix += (*face_verts).max(0) as usize;
    }
    if let Some(metrics) = profile.as_mut() {
        metrics.primvar_expansion_ms += primvar_start.elapsed().as_secs_f64() * 1000.0;
    }

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
        metrics.topology_triangulation_ms += triangulation_start.elapsed().as_secs_f64() * 1000.0;
    }

    let emit_normals = want_normals || smooth_per_point.is_some();
    (
        positions,
        emit_normals.then_some(normals_out),
        uvs_out,
        want_colors.then_some(colors_out),
        indices,
    )
}
