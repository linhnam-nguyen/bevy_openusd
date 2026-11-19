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
        indices: if indexed_uv {
            vec![0, 1, 2]
        } else {
            Vec::new()
        },
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

#[test]
fn reads_all_five_primvar_interpolations() {
    let stage = usd_support::open_fixture_stage("primvar_interp.usda");
    let cases = [
        ("/World/Constant", Interpolation::Constant, 1),
        ("/World/Uniform", Interpolation::Uniform, 2),
        ("/World/Vertex", Interpolation::Vertex, 4),
        ("/World/Varying", Interpolation::Varying, 4),
        ("/World/FaceVarying", Interpolation::FaceVarying, 4),
    ];
    for (path_str, expected_interp, expected_count) in cases {
        let path = openusd::sdf::path(path_str).expect("valid path");
        let mesh = crate::read::geom::read_mesh(&stage, &path)
            .expect("read ok")
            .expect("mesh decodes");
        let dc = mesh
            .display_color
            .as_ref()
            .unwrap_or_else(|| panic!("{path_str} should have displayColor"));
        assert_eq!(dc.interpolation, expected_interp);
        assert_eq!(dc.values.len(), expected_count);
    }
}

#[test]
fn mesh_from_usd_materialises_all_five_into_bevy_mesh_colors() {
    use bevy::mesh::{Mesh, VertexAttributeValues};

    let stage = usd_support::open_fixture_stage("primvar_interp.usda");
    let colors_of = |prim_path: &str| -> Vec<[f32; 4]> {
        let read = crate::read::geom::read_mesh(&stage, &openusd::sdf::path(prim_path).unwrap())
            .unwrap()
            .unwrap();
        let mesh = mesh_from_usd(&read);
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(v)) => v.clone(),
            other => panic!("expected Float32x4 ATTRIBUTE_COLOR on {prim_path}, got {other:?}"),
        }
    };

    let constant = colors_of("/World/Constant");
    assert!(!constant.is_empty());
    for c in &constant {
        assert!((c[0] - 1.0).abs() < 1e-4 && c[1].abs() < 1e-4 && c[2].abs() < 1e-4);
    }

    let vertex = colors_of("/World/Vertex");
    assert_eq!(vertex.len(), 4);
    assert!((vertex[0][0] - 1.0).abs() < 1e-4);
    assert!((vertex[1][1] - 1.0).abs() < 1e-4);
    assert!((vertex[2][2] - 1.0).abs() < 1e-4);

    let uniform = colors_of("/World/Uniform");
    assert_eq!(uniform.len(), 8);
    for c in &uniform[0..4] {
        assert!((c[0] - 1.0).abs() < 1e-4 && c[1].abs() < 1e-4);
    }
    for c in &uniform[4..8] {
        assert!(c[0].abs() < 1e-4 && (c[2] - 1.0).abs() < 1e-4);
    }

    let face_varying = colors_of("/World/FaceVarying");
    assert_eq!(face_varying.len(), 4);
    assert!((face_varying[0][0] - 1.0).abs() < 1e-4);
    assert!((face_varying[1][1] - 1.0).abs() < 1e-4);
    assert!((face_varying[2][2] - 1.0).abs() < 1e-4);
}
