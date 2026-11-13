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
