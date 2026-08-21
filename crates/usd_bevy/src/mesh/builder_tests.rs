use super::*;
use crate::read::geom::{Interpolation, MeshPrimvar, Orientation, ReadMesh, SubdivScheme};

fn triangle() -> ReadMesh {
    ReadMesh {
        points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        face_vertex_counts: vec![3],
        face_vertex_indices: vec![0, 1, 2],
        normals: None,
        uvs: None,
        orientation: Orientation::RightHanded,
        display_color: None,
        display_opacity: None,
        subsets: Vec::new(),
        double_sided: false,
        extent: None,
        subdivision_scheme: SubdivScheme::None,
    }
}

fn expanded_triangle(indexed_uv: bool) -> ReadMesh {
    let mut mesh = triangle();
    mesh.uvs = Some(MeshPrimvar {
        values: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        interpolation: Interpolation::FaceVarying,
        indices: indexed_uv.then(|| vec![0, 1, 2]).unwrap_or_default(),
    });
    mesh
}

#[test]
fn profiled_conversion_reports_source_and_output_counts() {
    let (mesh, metrics) = mesh_from_usd_profiled(&triangle());

    assert_eq!(metrics.source_points, 3);
    assert_eq!(metrics.source_faces, 1);
    assert_eq!(metrics.source_face_corners, 3);
    assert_eq!(metrics.output_vertices, 3);
    assert_eq!(metrics.output_indices, 3);
    assert_eq!(metrics.output_triangles, 1);
    assert!(!metrics.authored_normals);
    assert!(metrics.generated_normals);
    assert!(!metrics.expanded_vertices);
    assert_eq!(mesh.count_vertices(), 3);
}

#[test]
fn expanded_missing_authored_normals_keep_source_provenance() {
    let (mesh, metrics) = mesh_from_usd_profiled(&expanded_triangle(false));

    assert!(metrics.expanded_vertices);
    assert_eq!(metrics.output_vertices, 3);
    assert!(!metrics.authored_normals);
    assert!(metrics.generated_normals);
    assert_eq!(metrics.indexed_primvars, 0);
    assert_eq!(metrics.non_indexed_primvars, 1);
    assert_eq!(metrics.expansion_forcing_primvars, 1);
    assert_eq!(mesh.count_vertices(), 3);
}

#[test]
fn indexed_primvar_count_uses_source_indices_not_interpolation() {
    let (_, metrics) = mesh_from_usd_profiled(&expanded_triangle(true));

    assert_eq!(metrics.indexed_primvars, 1);
    assert_eq!(metrics.non_indexed_primvars, 0);
    assert_eq!(metrics.expansion_forcing_primvars, 1);
}
