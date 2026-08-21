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
            ..Default::default()
        });
    }

    assert_eq!(profile.totals.mesh_count, 3);
    assert_eq!(profile.totals.source_points, 30);
    assert_eq!(profile.records.len(), 2);
    assert_eq!(profile.records[0].read_mesh_ms, 3.0);
    assert_eq!(profile.records[1].read_mesh_ms, 2.0);
}
