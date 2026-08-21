use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use super::index::PrimEntities;
use super::progressive_state::ProgressiveProjectionState;

pub(super) fn resident_projection(
    world: &World,
    map: &PrimEntities,
    state: &ProgressiveProjectionState,
) -> bool {
    let Some(plan) = state.plan() else {
        return false;
    };
    plan.entries().all(|entry| {
        let Some(entity) = map.entity(entry.path()) else {
            return false;
        };
        let Some(prim) = world.get::<crate::prim_ref::UsdPrimRef>(entity) else {
            return false;
        };
        if prim.path != entry.path() {
            return false;
        }
        if let Some(mesh) = world.get::<Mesh3d>(entity)
            && let Some(assets) = world.get_resource::<Assets<Mesh>>()
            && !assets.contains(&mesh.0)
        {
            return false;
        }
        if let Some(material) = world.get::<MeshMaterial3d<StandardMaterial>>(entity)
            && let Some(assets) = world.get_resource::<Assets<StandardMaterial>>()
            && !assets.contains(&material.0)
        {
            return false;
        }
        true
    })
}
