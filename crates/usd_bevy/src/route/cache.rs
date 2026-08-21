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

use bevy::mesh::VertexAttributeValues;
use bevy::platform::hash::FixedHasher;
use bevy::prelude::*;

/// Upper bound on distinct interned meshes. The cache holds *strong* handles
/// (that's what keeps a shared mesh alive), so it must be bounded or a scene
/// that keeps minting distinct geometry — e.g. a time-varying `Points` prim —
/// would pin every past version alive. On overflow the map is cleared, which
/// drops those strong refs so `Assets<Mesh>` can reclaim anything unreferenced;
/// still-referenced meshes simply get re-interned on their next projection.
pub(crate) const MAX_INTERNED: usize = 8192;

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
    source_meshes: HashMap<u64, Handle<Mesh>>,
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
    let is_alive = existing
        .as_ref()
        .is_some_and(|handle| world.resource::<Assets<Mesh>>().contains(handle));
    {
        let mut cache = world.resource_mut::<ProjectionCache>();
        cache.stats.lookups += 1;
        if is_alive {
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
        if existing.is_some() {
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
        cache.source_meshes.clear();
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
pub(crate) fn lookup_source_mesh(world: &mut World, key: u64) -> Option<Handle<Mesh>> {
    let existing = world
        .resource::<ProjectionCache>()
        .source_meshes
        .get(&key)
        .cloned();
    let Some(handle) = existing else {
        return None;
    };
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
    cache.source_meshes.remove(&key);
    None
}

/// Remember the source key after a mesh has been built or found by the final
/// geometry interner. Both source and final caches share the same bound.
pub(crate) fn remember_source_mesh(world: &mut World, key: u64, handle: Handle<Mesh>) {
    let mut cache = world.resource_mut::<ProjectionCache>();
    if cache.source_meshes.len() >= MAX_INTERNED {
        cache.meshes.clear();
        cache.source_meshes.clear();
        cache.stats.evictions += 1;
    }
    cache.source_meshes.insert(key, handle);
}

/// A 64-bit signature over the geometry that defines a mesh's appearance:
/// topology, indices, and every attribute the mesh builder emits
/// (position / normal / uv / vertex color). Two meshes with equal signatures
/// render identically, so they can share one handle. Float lanes are hashed by
/// bit pattern (exact-equality — no fuzzy matching). Any attribute added here
/// MUST be one the builder actually produces, else the signature is stable but
/// blind to a difference and two visually distinct meshes could alias.
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
    for id in [
        Mesh::ATTRIBUTE_POSITION.id,
        Mesh::ATTRIBUTE_NORMAL.id,
        Mesh::ATTRIBUTE_UV_0.id,
        Mesh::ATTRIBUTE_COLOR.id,
    ] {
        hash_attribute(mesh, id, &mut h);
    }
    h.finish()
}

fn hash_attribute(mesh: &Mesh, id: bevy::mesh::MeshVertexAttributeId, h: &mut impl Hasher) {
    let Some(values) = mesh
        .attributes()
        .find(|(attr, _)| attr.id == id)
        .map(|(_, v)| v)
    else {
        0u8.hash(h);
        return;
    };
    match values {
        VertexAttributeValues::Float32x3(v) => {
            for a in v {
                for f in a {
                    f.to_bits().hash(h);
                }
            }
        }
        VertexAttributeValues::Float32x2(v) => {
            for a in v {
                for f in a {
                    f.to_bits().hash(h);
                }
            }
        }
        VertexAttributeValues::Float32x4(v) => {
            for a in v {
                for f in a {
                    f.to_bits().hash(h);
                }
            }
        }
        _ => {
            // Attribute present but a variant we don't fold in: fold its length
            // so meshes differing only there still get distinct signatures.
            values.len().hash(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

    use super::*;

    fn triangle() -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        mesh
    }

    #[test]
    fn stats_distinguish_projection_hit_and_miss() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.insert_resource(ProjectionCache::default());

        let first = intern_mesh(&mut world, triangle());
        let second = intern_mesh(&mut world, triangle());

        assert_eq!(first, second);
        assert_eq!(
            world.resource::<ProjectionCache>().stats(),
            ProjectionCacheStats {
                lookups: 2,
                hits: 1,
                misses: 1,
                stale_handles: 0,
                evictions: 0,
            }
        );
    }

    #[test]
    fn stale_handle_is_rebuilt_and_counted() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.insert_resource(ProjectionCache::default());

        let first = intern_mesh(&mut world, triangle());
        world.resource_mut::<Assets<Mesh>>().remove(first.id());
        let second = intern_mesh(&mut world, triangle());

        assert_ne!(first, second);
        let stats = world.resource::<ProjectionCache>().stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.stale_handles, 1);
    }
}
