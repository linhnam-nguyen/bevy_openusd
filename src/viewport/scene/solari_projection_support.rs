//! Small, pure helpers for the incremental Solari projection state.

#[cfg(feature = "solari")]
use bevy::mesh::{Mesh, Mesh3d};
#[cfg(feature = "solari")]
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
#[cfg(feature = "solari")]
use bevy::prelude::*;

#[cfg(feature = "solari")]
use super::{
    SolariCapability, SolariProjectionDiagnostics, SolariProjectionEntry, SolariProjectionState,
};

#[cfg(feature = "solari")]
use usd_bevy::UsdPrimRef;

#[cfg(feature = "solari")]
pub(super) fn evaluate_mesh(
    mesh_handle: &Mesh3d,
    material: Option<&MeshMaterial3d<StandardMaterial>>,
    meshes: &Assets<Mesh>,
) -> (bool, bool) {
    let missing_material = material.is_none();
    let eligible = material.is_some()
        && meshes
            .get(&mesh_handle.0)
            .is_some_and(mesh_is_solari_compatible);
    (eligible, missing_material)
}

#[cfg(feature = "solari")]
fn mesh_is_solari_compatible(mesh: &Mesh) -> bool {
    mesh.primitive_topology() == bevy::mesh::PrimitiveTopology::TriangleList
        && matches!(mesh.indices(), Some(bevy::mesh::Indices::U32(_)))
        && mesh.contains_attribute(Mesh::ATTRIBUTE_POSITION)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_NORMAL)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_UV_0)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT)
}

#[cfg(feature = "solari")]
pub(super) fn adjust_diagnostics(
    diagnostics: &mut SolariProjectionDiagnostics,
    entry: &SolariProjectionEntry,
    delta: i32,
) {
    adjust_counter(&mut diagnostics.candidate_meshes, delta);
    adjust_counter(
        &mut diagnostics.eligible_meshes,
        delta * i32::from(entry.eligible),
    );
    adjust_counter(
        &mut diagnostics.unsupported_meshes,
        delta * i32::from(!entry.eligible),
    );
    adjust_counter(
        &mut diagnostics.missing_materials,
        delta * i32::from(entry.missing_material),
    );
}

#[cfg(feature = "solari")]
fn adjust_counter(counter: &mut u32, delta: i32) {
    if delta >= 0 {
        *counter += delta as u32;
    } else {
        *counter -= delta.unsigned_abs();
    }
}

#[cfg(feature = "solari")]
pub(super) fn set_scene_eligibility(
    capability: &mut SolariCapability,
    diagnostics: &SolariProjectionDiagnostics,
) {
    let eligible = diagnostics.candidate_meshes > 0
        && diagnostics.eligible_meshes == diagnostics.candidate_meshes;
    if capability.scene_eligible != eligible {
        capability.scene_eligible = eligible;
    }
}

#[cfg(feature = "solari")]
pub(super) fn apply_marker(
    entity: Entity,
    entry: &mut SolariProjectionEntry,
    enabled: bool,
    commands: &mut Commands,
) {
    let desired = enabled && entry.eligible;
    if desired && !entry.raytracing_marker {
        commands
            .entity(entity)
            .insert(bevy::solari::prelude::RaytracingMesh3d(entry.mesh.clone()));
        entry.raytracing_marker = true;
    } else if !desired && entry.raytracing_marker {
        commands
            .entity(entity)
            .remove::<bevy::solari::prelude::RaytracingMesh3d>();
        entry.raytracing_marker = false;
    }
}

#[cfg(feature = "solari")]
pub(super) fn disable_projection(
    capability: &mut SolariCapability,
    state: &mut SolariProjectionState,
    diagnostics: &mut SolariProjectionDiagnostics,
    commands: &mut Commands,
    marked_entities: &mut Query<
        Entity,
        (
            With<UsdPrimRef>,
            With<bevy::solari::prelude::RaytracingMesh3d>,
        ),
    >,
) {
    for entry in state.entries.values_mut() {
        if entry.raytracing_marker {
            entry.raytracing_marker = false;
        }
    }
    for entity in &mut *marked_entities {
        commands
            .entity(entity)
            .remove::<bevy::solari::prelude::RaytracingMesh3d>();
    }
    if !state.entries.is_empty() || state.initialized || state.enabled {
        state.entries.clear();
        *diagnostics = SolariProjectionDiagnostics::default();
    }
    if state.initialized {
        state.initialized = false;
    }
    if state.device_supported {
        state.device_supported = false;
    }
    if state.requested_ray_traced {
        state.requested_ray_traced = false;
    }
    if state.enabled {
        state.enabled = false;
    }
    if capability.scene_eligible {
        capability.scene_eligible = false;
    }
}
