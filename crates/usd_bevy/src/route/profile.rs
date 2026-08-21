//! Opt-in geometry profiling for the USD mesh projection pipeline.
//!
//! The profiler is deliberately a resource rather than a logging side effect:
//! benchmark runners can snapshot deterministic counts and bounded expensive
//! prim records without changing the normal projection path.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// One profiled USD mesh conversion.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GeometryProfileRecord {
    pub read_mesh_ms: f64,
    pub mesh_from_usd_ms: f64,
    pub topology_triangulation_ms: f64,
    pub primvar_expansion_ms: f64,
    pub normal_generation_ms: f64,
    pub bevy_mesh_allocation_ms: f64,
    pub mesh_signature_ms: f64,
    pub mesh_intern_ms: f64,
    pub source_points: usize,
    pub source_faces: usize,
    pub source_face_corners: usize,
    pub output_vertices: usize,
    pub output_indices: usize,
    pub output_triangles: usize,
    pub authored_normals: bool,
    pub generated_normals: bool,
    pub expanded_vertices: bool,
    pub cache_hit: bool,
}

impl GeometryProfileRecord {
    /// End-to-end time attributed to reading, building, and interning.
    pub fn total_ms(&self) -> f64 {
        self.read_mesh_ms + self.mesh_from_usd_ms + self.mesh_intern_ms
    }
}

/// Deterministic aggregate counters for one profiling run.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GeometryProfileTotals {
    pub mesh_count: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub read_mesh_ms: f64,
    pub mesh_from_usd_ms: f64,
    pub topology_triangulation_ms: f64,
    pub primvar_expansion_ms: f64,
    pub normal_generation_ms: f64,
    pub bevy_mesh_allocation_ms: f64,
    pub mesh_signature_ms: f64,
    pub mesh_intern_ms: f64,
    pub source_points: usize,
    pub source_faces: usize,
    pub source_face_corners: usize,
    pub output_vertices: usize,
    pub output_indices: usize,
    pub output_triangles: usize,
}

/// Opt-in bounded geometry profiler.
#[derive(Resource, Debug, Clone, Deserialize, Serialize)]
pub struct GeometryProfile {
    /// False by default so ordinary projection pays no timing or bookkeeping cost.
    pub enabled: bool,
    /// Maximum number of expensive records retained in memory.
    pub top_n: usize,
    pub totals: GeometryProfileTotals,
    pub records: Vec<GeometryProfileRecord>,
}

impl Default for GeometryProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            top_n: 64,
            totals: GeometryProfileTotals::default(),
            records: Vec::new(),
        }
    }
}

impl GeometryProfile {
    pub fn reset(&mut self) {
        self.totals = GeometryProfileTotals::default();
        self.records.clear();
    }

    pub fn record(&mut self, sample: GeometryProfileRecord) {
        let totals = &mut self.totals;
        totals.mesh_count += 1;
        totals.cache_hits += usize::from(sample.cache_hit);
        totals.cache_misses += usize::from(!sample.cache_hit);
        totals.read_mesh_ms += sample.read_mesh_ms;
        totals.mesh_from_usd_ms += sample.mesh_from_usd_ms;
        totals.topology_triangulation_ms += sample.topology_triangulation_ms;
        totals.primvar_expansion_ms += sample.primvar_expansion_ms;
        totals.normal_generation_ms += sample.normal_generation_ms;
        totals.bevy_mesh_allocation_ms += sample.bevy_mesh_allocation_ms;
        totals.mesh_signature_ms += sample.mesh_signature_ms;
        totals.mesh_intern_ms += sample.mesh_intern_ms;
        totals.source_points += sample.source_points;
        totals.source_faces += sample.source_faces;
        totals.source_face_corners += sample.source_face_corners;
        totals.output_vertices += sample.output_vertices;
        totals.output_indices += sample.output_indices;
        totals.output_triangles += sample.output_triangles;

        if self.top_n == 0 {
            return;
        }
        self.records.push(sample);
        self.records.sort_by(|left, right| {
            right
                .total_ms()
                .total_cmp(&left.total_ms())
                .then_with(|| right.output_vertices.cmp(&left.output_vertices))
        });
        self.records.truncate(self.top_n);
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
