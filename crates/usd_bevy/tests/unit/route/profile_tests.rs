use super::*;

#[test]
fn aggregates_all_samples_and_keeps_bounded_expensive_records() {
    let mut profile = GeometryProfile {
        enabled: true,
        top_n: 2,
        ..Default::default()
    };
    for total in [1.0, 3.0, 2.0] {
        profile.record(GeometryProfileRecord {
            read_mesh_ms: total,
            source_points: 10,
            output_vertices: total as usize,
            mesh_conversion: true,
            ..Default::default()
        });
    }

    assert_eq!(profile.totals.mesh_count, 3);
    assert_eq!(profile.totals.source_points, 30);
    assert_eq!(profile.records.len(), 2);
    assert_eq!(profile.records[0].read_mesh_ms, 3.0);
    assert_eq!(profile.records[1].read_mesh_ms, 2.0);
}

#[test]
fn characterizes_topology_and_primvar_categories_without_allocating_labels() {
    assert_eq!(classify_topology(&[3, 3]), GeometryTopologyClass::Triangles);
    assert_eq!(classify_topology(&[4, 4]), GeometryTopologyClass::Quads);
    assert_eq!(classify_topology(&[3, 5]), GeometryTopologyClass::Mixed);

    let mut profile = GeometryProfile::default();
    profile.record(GeometryProfileRecord {
        authored_normals: true,
        uv_interpolation: GeometryInterpolation::FaceVarying,
        indexed_primvars: 1,
        non_indexed_primvars: 2,
        expansion_forcing_primvars: 2,
        display_color: true,
        topology_class: GeometryTopologyClass::Quads,
        subdivision: GeometrySubdivisionClass::CatmullClark,
        vertex_source_ratio: 2.0,
        ..Default::default()
    });

    assert_eq!(profile.totals.authored_normal_meshes, 1);
    assert_eq!(profile.totals.indexed_primvars, 1);
    assert_eq!(profile.totals.non_indexed_primvars, 2);
    assert_eq!(profile.totals.expansion_forcing_primvars, 2);
    assert_eq!(profile.totals.topology_counts[2], 1);
    assert_eq!(profile.totals.subdivision_counts[1], 1);
    assert_eq!(profile.totals.uv_interpolation_counts[5], 1);
    assert_eq!(profile.totals.vertex_source_ratio_sum, 2.0);
}

#[test]
fn top_records_use_stable_redaction_safe_identity_and_reason_bits() {
    assert_eq!(hash_prim_path("/World/Mesh"), hash_prim_path("/World/Mesh"));
    assert_ne!(
        hash_prim_path("/World/Mesh"),
        hash_prim_path("/World/Other")
    );

    let mut profile = GeometryProfile::default();
    profile.record(GeometryProfileRecord {
        prim_path_hash: hash_prim_path("/World/Mesh"),
        reason_flags: REASON_EXPANDED_PRIMVARS | REASON_CACHE_MISS,
        read_mesh_ms: 1.0,
        ..Default::default()
    });

    assert_eq!(
        profile.records[0].prim_path_hash,
        hash_prim_path("/World/Mesh")
    );
    assert_ne!(
        profile.records[0].reason_flags & REASON_EXPANDED_PRIMVARS,
        0
    );
    assert_ne!(profile.records[0].reason_flags & REASON_CACHE_MISS, 0);
}
