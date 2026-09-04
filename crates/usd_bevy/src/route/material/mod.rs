//! Material route: a gprim's bound `UsdShade` Material → [`StandardMaterial`].
//!
//! Runs after the mesh route, replacing the placeholder material the mesh route
//! attaches. Reads the `material:binding` and decodes the bound
//! `UsdPreviewSurface` (and Omni/MaterialX equivalents) via [`crate::read::shade`].
//! Resolves textures from the filesystem or embedded `.usdz` archives into [`Assets<Image>`].

mod archive;
mod builder;
mod consumers;
mod material_cache;
mod provenance;
mod texture_cache;

pub use builder::MaterialRoute;
pub(crate) use consumers::MaterialConsumerIndex;
pub use material_cache::{MaterialCacheStats, UsdMaterialCache};
pub use provenance::{MaterialProjectionProvenance, MaterialProjectionStatus};
pub use texture_cache::{TextureCacheKey, TextureCacheStats, UsdTextureCache};

pub(crate) fn cleanup_retired_materials(world: &mut bevy::ecs::world::World) {
    material_cache::cleanup_retired_materials(world);
}

/// Route-level counters kept separate from cache counters so profiling can
/// distinguish route dispatch from material/texture reuse.
#[derive(bevy::ecs::resource::Resource, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaterialRouteDiagnostics {
    pub matches: u64,
    pub projects: u64,
    pub patches: u64,
    pub descriptor_reads: u64,
}

pub(crate) fn record_match(world: &mut bevy::ecs::world::World) {
    world.init_resource::<MaterialRouteDiagnostics>();
    world.resource_mut::<MaterialRouteDiagnostics>().matches += 1;
}

pub(crate) fn record_project(world: &mut bevy::ecs::world::World) {
    world.init_resource::<MaterialRouteDiagnostics>();
    world.resource_mut::<MaterialRouteDiagnostics>().projects += 1;
}

pub(crate) fn record_patch(world: &mut bevy::ecs::world::World) {
    world.init_resource::<MaterialRouteDiagnostics>();
    world.resource_mut::<MaterialRouteDiagnostics>().patches += 1;
}

pub(crate) fn record_descriptor_read(world: &mut bevy::ecs::world::World) {
    world.init_resource::<MaterialRouteDiagnostics>();
    world
        .resource_mut::<MaterialRouteDiagnostics>()
        .descriptor_reads += 1;
}

#[cfg(test)]
mod fallback_tests;
#[cfg(test)]
mod tests;
