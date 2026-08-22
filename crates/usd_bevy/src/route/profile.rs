//! Opt-in geometry profiling for the USD mesh projection pipeline.
//!
//! The profiler is deliberately a resource rather than a logging side effect:
//! benchmark runners can snapshot deterministic counts and bounded expensive
//! prim records without changing the normal projection path.

use bevy::platform::hash::FixedHasher;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::hash::BuildHasher;

pub const REASON_EXPANDED_PRIMVARS: u32 = 1 << 0;
pub const REASON_GENERATED_NORMALS: u32 = 1 << 1;
pub const REASON_SUBDIVISION: u32 = 1 << 2;
pub const REASON_DISPLAY_COLOR: u32 = 1 << 3;
pub const REASON_DISPLAY_OPACITY: u32 = 1 << 4;
pub const REASON_CACHE_MISS: u32 = 1 << 5;
pub const REASON_HIGH_VERTEX_EXPANSION: u32 = 1 << 6;

/// Stable redaction-safe identity for a USD prim path.
pub fn hash_prim_path(path: &str) -> u64 {
    FixedHasher.hash_one(path)
}

#[allow(clippy::too_many_arguments)]
pub fn record_mesh_sample(
    profile: &mut GeometryProfile,
    prim_path_hash: u64,
    read_mesh_ms: f64,
    build: crate::mesh::MeshBuildMetrics,
    intern: crate::route::cache::MeshInternMetrics,
    mesh_conversion: bool,
    source_cache_lookup: bool,
    source_cache_hit: bool,
) {
    let mut reason_flags = 0;
    if build.expansion_forcing_primvars > 0 {
        reason_flags |= REASON_EXPANDED_PRIMVARS;
    }
    if build.generated_normals {
        reason_flags |= REASON_GENERATED_NORMALS;
    }
    if build.subdivision != GeometrySubdivisionClass::None {
        reason_flags |= REASON_SUBDIVISION;
    }
    if build.display_color {
        reason_flags |= REASON_DISPLAY_COLOR;
    }
    if build.display_opacity {
        reason_flags |= REASON_DISPLAY_OPACITY;
    }
    if intern.cache_lookup && !intern.cache_hit {
        reason_flags |= REASON_CACHE_MISS;
    }
    if build.vertex_source_ratio > 1.0 {
        reason_flags |= REASON_HIGH_VERTEX_EXPANSION;
    }
    profile.record(GeometryProfileRecord {
        prim_path_hash,
        read_mesh_ms,
        mesh_from_usd_ms: build.mesh_from_usd_ms,
        topology_triangulation_ms: build.topology_triangulation_ms,
        primvar_expansion_ms: build.primvar_expansion_ms,
        normal_generation_ms: build.normal_generation_ms,
        bevy_mesh_allocation_ms: intern.allocation_ms,
        mesh_signature_ms: intern.signature_ms,
        mesh_intern_ms: intern.total_ms,
        source_points: build.source_points,
        source_faces: build.source_faces,
        source_face_corners: build.source_face_corners,
        output_vertices: build.output_vertices,
        output_indices: build.output_indices,
        output_triangles: build.output_triangles,
        mesh_conversion,
        source_cache_lookup,
        source_cache_hit,
        cache_lookup: intern.cache_lookup,
        cache_hit: intern.cache_hit,
        authored_normals: build.authored_normals,
        generated_normals: build.generated_normals,
        expanded_vertices: build.expanded_vertices,
        uv_interpolation: build.uv_interpolation,
        indexed_primvars: build.indexed_primvars,
        non_indexed_primvars: build.non_indexed_primvars,
        expansion_forcing_primvars: build.expansion_forcing_primvars,
        display_color: build.display_color,
        display_opacity: build.display_opacity,
        topology_class: build.topology_class,
        subdivision: build.subdivision,
        vertex_source_ratio: build.vertex_source_ratio,
        reason_flags,
    });
}

/// Compact, allocation-free interpolation classification for profile output.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum GeometryInterpolation {
    #[default]
    Absent,
    Constant,
    Uniform,
    Varying,
    Vertex,
    FaceVarying,
}

/// Source topology classification before triangulation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum GeometryTopologyClass {
    #[default]
    Empty,
    Triangles,
    Quads,
    Ngons,
    Mixed,
}

/// Authored subdivision scheme classification.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum GeometrySubdivisionClass {
    #[default]
    None,
    CatmullClark,
    Loop,
    Bilinear,
}

impl From<crate::read::geom::Interpolation> for GeometryInterpolation {
    fn from(value: crate::read::geom::Interpolation) -> Self {
        match value {
            crate::read::geom::Interpolation::Constant => Self::Constant,
            crate::read::geom::Interpolation::Uniform => Self::Uniform,
            crate::read::geom::Interpolation::Varying => Self::Varying,
            crate::read::geom::Interpolation::Vertex => Self::Vertex,
            crate::read::geom::Interpolation::FaceVarying => Self::FaceVarying,
        }
    }
}

impl From<crate::read::geom::SubdivScheme> for GeometrySubdivisionClass {
    fn from(value: crate::read::geom::SubdivScheme) -> Self {
        match value {
            crate::read::geom::SubdivScheme::None => Self::None,
            crate::read::geom::SubdivScheme::CatmullClark => Self::CatmullClark,
            crate::read::geom::SubdivScheme::Loop => Self::Loop,
            crate::read::geom::SubdivScheme::Bilinear => Self::Bilinear,
        }
    }
}

pub fn classify_topology(counts: &[i32]) -> GeometryTopologyClass {
    let mut saw_triangles = false;
    let mut saw_quads = false;
    let mut saw_ngons = false;
    for &count in counts {
        match count {
            3 => saw_triangles = true,
            4 => saw_quads = true,
            0..=2 => {}
            _ => saw_ngons = true,
        }
    }
    match (saw_triangles, saw_quads, saw_ngons) {
        (false, false, false) => GeometryTopologyClass::Empty,
        (true, false, false) => GeometryTopologyClass::Triangles,
        (false, true, false) => GeometryTopologyClass::Quads,
        (false, false, true) => GeometryTopologyClass::Ngons,
        _ => GeometryTopologyClass::Mixed,
    }
}

/// One profiled USD mesh conversion.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GeometryProfileRecord {
    pub prim_path_hash: u64,
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
    /// True when `mesh_from_usd` actually converted the source read.
    pub mesh_conversion: bool,
    /// True when the pre-conversion source-content cache was consulted.
    pub source_cache_lookup: bool,
    /// True when the pre-conversion source-content cache supplied the mesh.
    pub source_cache_hit: bool,
    /// True when the final post-conversion interner was consulted.
    pub cache_lookup: bool,
    pub authored_normals: bool,
    pub generated_normals: bool,
    pub expanded_vertices: bool,
    pub cache_hit: bool,
    pub uv_interpolation: GeometryInterpolation,
    pub indexed_primvars: usize,
    pub non_indexed_primvars: usize,
    pub expansion_forcing_primvars: usize,
    pub display_color: bool,
    pub display_opacity: bool,
    pub topology_class: GeometryTopologyClass,
    pub subdivision: GeometrySubdivisionClass,
    pub vertex_source_ratio: f64,
    pub reason_flags: u32,
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
    /// Number of route samples, including source-cache hits.
    pub mesh_route_samples: usize,
    /// Number of actual `mesh_from_usd` conversions.
    pub mesh_count: usize,
    pub source_cache_lookups: usize,
    pub source_cache_hits: usize,
    pub source_cache_misses: usize,
    /// Final post-conversion interner counters.
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
    pub authored_normal_meshes: usize,
    pub generated_normal_meshes: usize,
    pub indexed_primvars: usize,
    pub non_indexed_primvars: usize,
    pub expansion_forcing_primvars: usize,
    pub display_color_meshes: usize,
    pub display_opacity_meshes: usize,
    pub vertex_source_ratio_sum: f64,
    pub topology_counts: [usize; 5],
    pub subdivision_counts: [usize; 4],
    pub uv_interpolation_counts: [usize; 6],
}

/// Opt-in bounded geometry profiler.
#[derive(Resource, Debug, Clone, Deserialize, Serialize, PartialEq)]
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
        totals.mesh_route_samples += 1;
        totals.mesh_count += usize::from(sample.mesh_conversion);
        totals.source_cache_lookups += usize::from(sample.source_cache_lookup);
        totals.source_cache_hits += usize::from(sample.source_cache_hit);
        totals.source_cache_misses +=
            usize::from(sample.source_cache_lookup && !sample.source_cache_hit);
        totals.cache_hits += usize::from(sample.cache_lookup && sample.cache_hit);
        totals.cache_misses += usize::from(sample.cache_lookup && !sample.cache_hit);
        totals.authored_normal_meshes += usize::from(sample.authored_normals);
        totals.generated_normal_meshes += usize::from(sample.generated_normals);
        totals.indexed_primvars += sample.indexed_primvars;
        totals.non_indexed_primvars += sample.non_indexed_primvars;
        totals.expansion_forcing_primvars += sample.expansion_forcing_primvars;
        totals.display_color_meshes += usize::from(sample.display_color);
        totals.display_opacity_meshes += usize::from(sample.display_opacity);
        totals.vertex_source_ratio_sum += sample.vertex_source_ratio;
        totals.topology_counts[topology_index(sample.topology_class)] += 1;
        totals.subdivision_counts[subdivision_index(sample.subdivision)] += 1;
        totals.uv_interpolation_counts[interpolation_index(sample.uv_interpolation)] += 1;
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

fn interpolation_index(value: GeometryInterpolation) -> usize {
    match value {
        GeometryInterpolation::Absent => 0,
        GeometryInterpolation::Constant => 1,
        GeometryInterpolation::Uniform => 2,
        GeometryInterpolation::Varying => 3,
        GeometryInterpolation::Vertex => 4,
        GeometryInterpolation::FaceVarying => 5,
    }
}

fn topology_index(value: GeometryTopologyClass) -> usize {
    match value {
        GeometryTopologyClass::Empty => 0,
        GeometryTopologyClass::Triangles => 1,
        GeometryTopologyClass::Quads => 2,
        GeometryTopologyClass::Ngons => 3,
        GeometryTopologyClass::Mixed => 4,
    }
}

fn subdivision_index(value: GeometrySubdivisionClass) -> usize {
    match value {
        GeometrySubdivisionClass::None => 0,
        GeometrySubdivisionClass::CatmullClark => 1,
        GeometrySubdivisionClass::Loop => 2,
        GeometrySubdivisionClass::Bilinear => 3,
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
