//! Extraction functions for renderer phase breakdown metrics and cache snapshots.

use bevy::prelude::*;
use usd_bevy::{AnimatedPrims, PrimEntities, ProjectionStats};

use super::aggregate::{CacheSnapshot, PhaseMetrics};

/// Optional detailed phase timing resource for fine-grained renderer diagnostics.
#[derive(Resource, Debug, Clone, Default)]
pub struct ProjectionPhaseTimings {
    pub mesh_generation_ms: Option<f64>,
    pub primvar_expansion_ms: Option<f64>,
    pub normal_generation_ms: Option<f64>,
    pub material_resolve_ms: Option<f64>,
}

/// Collects phase breakdown timings from the Bevy world.
pub fn collect_phase_metrics_from_world(world: &World) -> PhaseMetrics {
    let proj_stats = world.get_resource::<ProjectionStats>();
    let phase_timings = world.get_resource::<ProjectionPhaseTimings>();

    let initial_projection_ms = proj_stats.and_then(|s| s.initial_projection_ms);
    let initial_projection_prims = proj_stats.map(|s| s.initial_projection_prims).unwrap_or(0);
    let stage_traversal_ms = proj_stats.and_then(|s| s.stage_traversal_ms);

    let mesh_generation_ms = phase_timings.and_then(|t| t.mesh_generation_ms);
    let primvar_expansion_ms = phase_timings.and_then(|t| t.primvar_expansion_ms);
    let normal_generation_ms = phase_timings.and_then(|t| t.normal_generation_ms);
    let material_resolve_ms = phase_timings.and_then(|t| t.material_resolve_ms);

    PhaseMetrics {
        initial_projection_ms,
        initial_projection_prims,
        stage_traversal_ms,
        mesh_generation_ms,
        primvar_expansion_ms,
        normal_generation_ms,
        material_resolve_ms,
    }
}

/// Collects cache snapshot and live stage prim counts from the Bevy world.
pub fn collect_cache_snapshot_from_world(world: &World) -> CacheSnapshot {
    let prim_entities = world.get_resource::<PrimEntities>();
    let animated_prims = world.get_resource::<AnimatedPrims>();

    let live_stage_prims = prim_entities.map(|p| p.len() as u64).unwrap_or(0);
    let live_stage_animated_prims = animated_prims.map(|a| a.0.len() as u64).unwrap_or(0);

    let cached_materials = world
        .get_resource::<Assets<StandardMaterial>>()
        .map(|m| m.len() as u64)
        .unwrap_or(0);

    let cached_textures = world
        .get_resource::<Assets<Image>>()
        .map(|img| img.len() as u64)
        .unwrap_or(0);

    CacheSnapshot {
        live_stage_prims,
        live_stage_animated_prims,
        cached_materials,
        cached_textures,
        material_hits: 0,
        material_misses: 0,
        texture_hits: 0,
        texture_misses: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_snapshot_empty_behavior() {
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
        });

        let metrics = collect_phase_metrics_from_world(&world);
        assert_eq!(metrics.initial_projection_ms, Some(12.5));
        assert_eq!(metrics.initial_projection_prims, 34);
        assert_eq!(metrics.stage_traversal_ms, Some(8.2));
        assert_eq!(metrics.mesh_generation_ms, None);
    }
}
