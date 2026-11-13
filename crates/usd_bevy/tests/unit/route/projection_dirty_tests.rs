use super::*;
use bevy::asset::Assets;
use bevy::math::primitives::Cuboid;

#[test]
fn dirty_set_deduplicates_entities_until_consumed() {
    let first = Entity::from_raw_u32(1).unwrap();
    let second = Entity::from_raw_u32(2).unwrap();
    let mut dirty = RenderProjectionDirtySet::default();

    dirty.mark(first);
    dirty.mark(first);
    dirty.mark(second);

    assert_eq!(dirty.len(), 2);
    let drained = dirty.take();
    assert_eq!(drained.len(), 2);
    assert!(drained.contains(&first));
    assert!(drained.contains(&second));
    assert_eq!(dirty.len(), 0);
}

#[test]
fn mesh_consumer_index_tracks_replacements_and_removals() {
    let mut assets = Assets::<Mesh>::default();
    let first_mesh = assets.add(Mesh::from(Cuboid::default()));
    let second_mesh = assets.add(Mesh::from(Cuboid::default()));
    let first = Entity::from_raw_u32(1).unwrap();
    let second = Entity::from_raw_u32(2).unwrap();
    let mut consumers = MeshProjectionConsumers::default();

    assert!(consumers.track(first, first_mesh.id()));
    assert!(!consumers.track(first, first_mesh.id()));
    assert!(consumers.track(second, first_mesh.id()));
    assert_eq!(consumers.consumer_count(first_mesh.id()), 2);

    assert!(consumers.track(first, second_mesh.id()));
    assert_eq!(consumers.consumer_count(first_mesh.id()), 1);
    assert_eq!(consumers.consumer_count(second_mesh.id()), 1);

    consumers.remove(second);
    assert_eq!(consumers.consumer_count(first_mesh.id()), 0);
}
