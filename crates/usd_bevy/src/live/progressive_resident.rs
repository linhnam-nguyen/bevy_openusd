use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use super::index::PrimEntities;
use super::path::PathStore;
use super::progressive_state::ProgressiveProjectionState;

pub(super) fn resident_projection(
    world: &World,
    map: &PrimEntities,
    paths: &PathStore,
    state: &ProgressiveProjectionState,
) -> bool {
    let is_resident = |path: &str, entity: Entity| {
        let Some(prim) = world.get::<crate::prim_ref::UsdPrimRef>(entity) else {
            return false;
        };
        if prim.path != path {
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
    };
    if state.resident_paths_changed {
        // The initial plan has been superseded by a successful targeted
        // reconcile. Validate the current registered paths and handles.
        map.iter(paths)
            .all(|(path, entity)| is_resident(path, entity))
    } else {
        state.plan().is_some_and(|plan| {
            plan.entries().all(|entry| {
                map.entity(paths, entry.path())
                    .is_some_and(|entity| is_resident(entry.path(), entity))
            })
        })
    }
}
