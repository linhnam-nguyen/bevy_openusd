//! Projected-mesh cache (PLAN Phase 6d): BSN's copy-on-write analog for USD.
//!
//! Every mesh-producing route builds a fresh [`Mesh`] and would otherwise
//! `Assets::add` it on each projection — so N identical prototype prims (a
//! kitbashed scene) allocate N identical GPU meshes, and re-projecting the same
//! prim mints a new handle each time. This resource interns meshes by a hash of
//! their geometry: identical content resolves to one shared [`Handle<Mesh>`].
//!
//! The cache is **opt-in**: when the resource is absent (a bare test `World`)
//! [`intern_mesh`] falls back to a plain `add`, so routes work either way.
//! [`crate::UsdPlugin`] inserts it.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::Instant;

use super::cache_key::{MAX_SOURCE_KEY_BYTES, MeshSourceKey};
use bevy::platform::hash::FixedHasher;
use bevy::prelude::*;

/// Upper bound on distinct interned meshes. The cache holds *strong* handles
/// (that's what keeps a shared mesh alive), so it must be bounded or a scene
/// that keeps minting distinct geometry — e.g. a time-varying `Points` prim —
/// would pin every past version alive. On overflow the map is cleared, which
/// drops those strong refs so `Assets<Mesh>` can reclaim anything unreferenced;
/// still-referenced meshes simply get re-interned on their next projection.
pub(crate) const MAX_INTERNED: usize = 8192;
pub(crate) const MAX_SOURCE_MESHES: usize = 256;

/// Counters collected by [`ProjectionCache`] so cache policy changes can be
/// based on observed scene behavior rather than assumptions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectionCacheStats {
    pub lookups: u64,
    pub hits: u64,
    pub misses: u64,
    pub stale_handles: u64,
    pub evictions: u64,
}

/// Timings for one mesh interning operation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshInternMetrics {
    pub total_ms: f64,
    pub signature_ms: f64,
    pub allocation_ms: f64,
    pub cache_lookup: bool,
    pub cache_hit: bool,
}

/// Interns projected meshes by geometry signature so identical prims share one
/// [`Handle<Mesh>`]. Insert via [`crate::UsdPlugin`]; absent ⇒ no interning.
///
/// Interning is only worthwhile for geometry that repeats or persists (static
/// prototypes). Per-frame-unique geometry — notably CPU-skinned meshes, which
/// re-deform every time code — deliberately bypasses the cache (see
/// [`intern_mesh`]'s callers) so it neither bloats the map nor pins dead meshes.
#[derive(Resource, Default)]
pub struct ProjectionCache {
    meshes: HashMap<u64, Handle<Mesh>>,
    source_meshes: HashMap<MeshSourceKey, Handle<Mesh>>,
    source_key_bytes: usize,
    stats: ProjectionCacheStats,
}

impl ProjectionCache {
    /// Number of distinct meshes currently interned.
    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    /// Whether the cache holds no interned meshes.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// Number of source-read keys retained for pre-conversion reuse.
    pub fn source_len(&self) -> usize {
        self.source_meshes.len()
    }

    fn clear_source_keys(&mut self) {
        self.source_meshes.clear();
        self.source_key_bytes = 0;
    }

    /// Snapshot hit/miss and eviction counters for diagnostics or profiling.
    pub fn stats(&self) -> ProjectionCacheStats {
        self.stats
    }

    /// Clear profiling counters without dropping cached mesh handles.
    pub fn reset_stats(&mut self) {
        self.stats = ProjectionCacheStats::default();
    }
}

/// Add `mesh` to `Assets<Mesh>`, reusing an existing handle when a mesh with
/// identical geometry was already interned this session. Falls back to a plain
/// `add` when there is no [`ProjectionCache`] resource.
pub fn intern_mesh(world: &mut World, mesh: Mesh) -> Handle<Mesh> {
    intern_mesh_profiled(world, mesh).0
}

/// [`intern_mesh`] with timings for the optional geometry profiler.
pub fn intern_mesh_profiled(world: &mut World, mesh: Mesh) -> (Handle<Mesh>, MeshInternMetrics) {
    let total_start = Instant::now();
    // No cache resource → behave exactly like `Assets::add`.
    if world.get_resource::<ProjectionCache>().is_none() {
        let allocation_start = Instant::now();
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        return (
            handle,
            MeshInternMetrics {
                total_ms: total_start.elapsed().as_secs_f64() * 1000.0,
                allocation_ms: allocation_start.elapsed().as_secs_f64() * 1000.0,
                ..Default::default()
            },
        );
    }
    let signature_start = Instant::now();
    let sig = mesh_signature(&mesh);
    let signature_ms = signature_start.elapsed().as_secs_f64() * 1000.0;
    let existing = world
        .resource::<ProjectionCache>()
        .meshes
        .get(&sig)
        .cloned();
    let cached_mesh = existing
        .as_ref()
        .and_then(|handle| world.resource::<Assets<Mesh>>().get(handle));
    let is_alive = cached_mesh.is_some();
    let is_equal = cached_mesh.is_some_and(|cached| render_mesh_eq(cached, &mesh));
    {
        let mut cache = world.resource_mut::<ProjectionCache>();
        cache.stats.lookups += 1;
        if is_equal {
            cache.stats.hits += 1;
            return (
                existing.expect("alive cache entry must have a handle"),
                MeshInternMetrics {
                    total_ms: total_start.elapsed().as_secs_f64() * 1000.0,
                    signature_ms,
                    cache_lookup: true,
                    cache_hit: true,
                    ..Default::default()
                },
            );
        }
        if existing.is_some() && !is_alive {
            cache.stats.stale_handles += 1;
        }
        cache.stats.misses += 1;
    }
    let allocation_start = Instant::now();
    let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let allocation_ms = allocation_start.elapsed().as_secs_f64() * 1000.0;
    let mut cache = world.resource_mut::<ProjectionCache>();
    // Bound memory: clearing drops the strong handles so unreferenced meshes are
    // reclaimable. A stale (dead-handle) entry we passed over above also gets
    // swept here rather than lingering.
    if cache.meshes.len() >= MAX_INTERNED {
        cache.meshes.clear();
        cache.clear_source_keys();
        cache.stats.evictions += 1;
    }
    cache.meshes.insert(sig, handle.clone());
    (
        handle,
        MeshInternMetrics {
            total_ms: total_start.elapsed().as_secs_f64() * 1000.0,
            signature_ms,
            allocation_ms,
            cache_lookup: true,
            cache_hit: false,
        },
    )
}

/// Look up a source-read mesh before candidate mesh construction.
pub(crate) fn lookup_source_mesh(world: &mut World, key: &MeshSourceKey) -> Option<Handle<Mesh>> {
    let existing = world
        .resource::<ProjectionCache>()
        .source_meshes
        .get(key)
        .cloned();
    let handle = existing?;
    let alive = world
        .get_resource::<Assets<Mesh>>()
        .is_some_and(|assets| assets.contains(&handle));
    let mut cache = world.resource_mut::<ProjectionCache>();
    if alive {
        cache.stats.lookups += 1;
        cache.stats.hits += 1;
        return Some(handle);
    }
    cache.stats.stale_handles += 1;
    cache.source_meshes.remove(key);
    cache.source_key_bytes -= key.0.len();
    None
}

/// Remember the source key after a mesh has been built or found by the final
/// geometry interner. The source index is independently bounded by count and key bytes.
pub(crate) fn remember_source_mesh(world: &mut World, key: MeshSourceKey, handle: Handle<Mesh>) {
    let mut cache = world.resource_mut::<ProjectionCache>();
    let bytes = key.0.len();
    if bytes > MAX_SOURCE_KEY_BYTES {
        return;
    }
    if let Some(existing) = cache.source_meshes.get_mut(&key) {
        *existing = handle;
        return;
    }
    if cache.source_meshes.len() >= MAX_SOURCE_MESHES
        || bytes > MAX_SOURCE_KEY_BYTES - cache.source_key_bytes
    {
        return;
    }
    cache.source_key_bytes += bytes;
    cache.source_meshes.insert(key, handle);
}

/// A fast lookup hint. Reuse also checks the mesh and the exact vertex bytes.
fn mesh_signature(mesh: &Mesh) -> u64 {
    let mut h = FixedHasher.build_hasher();
    std::mem::discriminant(&mesh.primitive_topology()).hash(&mut h);
    match mesh.indices() {
        Some(bevy::mesh::Indices::U16(v)) => {
            0u8.hash(&mut h);
            v.hash(&mut h);
        }
        Some(bevy::mesh::Indices::U32(v)) => {
            1u8.hash(&mut h);
            v.hash(&mut h);
        }
        None => 2u8.hash(&mut h),
    }
    for (attribute, values) in mesh.attributes() {
        attribute.id.hash(&mut h);
        attribute.format.hash(&mut h);
        values.get_bytes().hash(&mut h);
    }
    h.finish()
}

fn render_mesh_eq(cached: &Mesh, requested: &Mesh) -> bool {
    cached == requested
        && cached.try_attributes().is_ok_and(|mut attributes| {
            attributes.all(|(attribute, values)| {
                requested
                    .attribute(attribute.id)
                    .is_some_and(|other| values.get_bytes() == other.get_bytes())
            })
        })
}

#[cfg(test)]
#[path = "../../tests/unit/route/cache_tests.rs"]
mod tests;
