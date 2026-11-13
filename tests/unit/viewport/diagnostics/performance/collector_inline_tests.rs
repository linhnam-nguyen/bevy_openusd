use super::*;

#[test]
fn cache_snapshot_empty_world() {
    let world = World::new();
    let snapshot = collect_cache_snapshot_from_world(&world);
    assert_eq!(snapshot.live_stage_prims, 0);
    assert_eq!(snapshot.live_stage_animated_prims, 0);
    assert_eq!(snapshot.cached_materials, 0);
    assert_eq!(snapshot.cached_textures, 0);
}

#[test]
fn phase_metrics_conversion_and_fallback() {
    let mut world = World::new();
    world.insert_resource(ProjectionStats {
        initial_projection_ms: Some(12.5),
        initial_projection_prims: 34,
        stage_traversal_ms: Some(8.2),
        mesh_generation_ms: Some(2.1),
        primvar_expansion_ms: Some(0.8),
        normal_generation_ms: Some(0.7),
        material_resolve_ms: Some(0.7),
    });

    let metrics = collect_phase_metrics_from_world(&world);
    assert_eq!(metrics.initial_projection_ms, Some(12.5));
    assert_eq!(metrics.initial_projection_prims, 34);
    assert_eq!(metrics.stage_traversal_ms, Some(8.2));
    assert_eq!(metrics.mesh_generation_ms, Some(2.1));
}
