use bevy::prelude::*;

use super::index::PrimEntities;
use super::native_instance_dependency::NativeInstanceDependencyIndex;
use crate::prim_ref::SemanticEntityIndex;

pub(super) fn clear_projection(world: &mut World, map: &mut PrimEntities) {
    if let Some(mut dependencies) = world.get_resource_mut::<NativeInstanceDependencyIndex>() {
        dependencies.clear();
    }
    let mut entities: Vec<(String, Entity)> = map
        .iter()
        .map(|(path, entity)| (path.to_string(), entity))
        .collect();
    entities.sort_by(|(left, _), (right, _)| {
        right
            .matches('/')
            .count()
            .cmp(&left.matches('/').count())
            .then_with(|| right.cmp(left))
    });
    for (path, entity) in entities {
        if let Some(mut semantic) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic.remove_entity(entity);
        }
        if let Some(mut materials) =
            world.get_resource_mut::<crate::route::material::MaterialConsumerIndex>()
        {
            materials.remove_consumer(&path);
        }
        world.despawn(entity);
        map.remove_path(&path);
    }
}
