use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CycleSample {
    pub cycle: usize,
    pub fixture: String,
    pub session_id: u64,
    pub resize_generation: u64,
    pub resize_width: u32,
    pub resize_height: u32,
    pub projected_prims: usize,
    pub mesh_assets: usize,
    pub material_assets: usize,
    pub image_assets: usize,
    pub projection_cache_meshes: usize,
    pub projection_cache_sources: usize,
    pub material_cache_entries: usize,
    pub texture_cache_entries: usize,
    pub point_instancer_full_projects: u64,
    pub point_instancer_sparse_transform_patches: u64,
    pub point_instancer_spawns: u64,
    pub point_instancer_despawns: u64,
    pub projection_ms: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct BoundSummary {
    pub metric: &'static str,
    pub all_cycles_min: usize,
    pub all_cycles_max: usize,
    pub steady_cycles_min: usize,
    pub steady_cycles_max: usize,
    pub bounded: bool,
}

#[derive(Debug, Serialize)]
pub struct PersistentSoakArtifact {
    pub schema: &'static str,
    pub checkpoint: &'static str,
    pub build_profile: &'static str,
    pub process_id: u32,
    pub cycle_count: usize,
    pub persistent_app: bool,
    pub workload_sequence: Vec<&'static str>,
    pub bounds: Vec<BoundSummary>,
    pub samples: Vec<CycleSample>,
    pub passed: bool,
}
