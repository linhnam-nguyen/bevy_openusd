use super::*;
use crate::read::geom::{Orientation, ReadMesh, SubdivScheme};

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
