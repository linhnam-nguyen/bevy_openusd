//! Incremental USD projection eligibility and Solari marker synchronization.

use std::collections::HashMap;

#[cfg(feature = "solari")]
use std::collections::HashSet;

#[cfg(feature = "solari")]
use bevy::asset::AssetEvent;
use bevy::mesh::Mesh;
#[cfg(feature = "solari")]
use bevy::mesh::Mesh3d;
#[cfg(feature = "solari")]
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
#[cfg(feature = "solari")]
use usd_bevy::{MeshProjectionConsumers, RenderProjectionDirtySet, UsdPrimRef};

#[cfg(feature = "solari")]
use super::SolariCapability;
#[cfg(feature = "solari")]
use crate::viewport::scene::visualization::DisplayToggles;

#[path = "solari_projection_support.rs"]
mod support;
#[cfg(feature = "solari")]
use support::{
    adjust_diagnostics, apply_marker, disable_projection, evaluate_mesh, set_scene_eligibility,
};

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariProjectionDiagnostics {
    pub(crate) candidate_meshes: u32,
    pub(crate) eligible_meshes: u32,
    pub(crate) unsupported_meshes: u32,
    pub(crate) missing_materials: u32,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct SolariProjectionState {
    initialized: bool,
    device_supported: bool,
    requested_ray_traced: bool,
    enabled: bool,
    entries: HashMap<Entity, SolariProjectionEntry>,
}

#[derive(Debug, Clone)]
struct SolariProjectionEntry {
    mesh: Handle<Mesh>,
    eligible: bool,
    missing_material: bool,
    raytracing_marker: bool,
}

#[cfg(test)]
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariProjectionStats {
    pub(crate) full_scans: u32,
    pub(crate) incremental_entities: u32,
}

#[cfg(test)]
impl SolariProjectionState {
    pub(crate) fn initialized_for_test() -> Self {
        Self {
            initialized: true,
            device_supported: true,
            ..Default::default()
        }
    }
}

#[cfg(feature = "solari")]
pub(super) fn sync_solari_usd_projection(
    mut capability: ResMut<SolariCapability>,
    toggles: Res<DisplayToggles>,
    meshes: Option<Res<Assets<Mesh>>>,
    mut state: ResMut<SolariProjectionState>,
    mut diagnostics: ResMut<SolariProjectionDiagnostics>,
    mut commands: Commands,
    mut projection_query: Query<
        (
            Entity,
            &Mesh3d,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&bevy::solari::prelude::RaytracingMesh3d>,
        ),
        With<UsdPrimRef>,
    >,
    mut marked_entities: Query<
        Entity,
        (
            With<UsdPrimRef>,
            With<bevy::solari::prelude::RaytracingMesh3d>,
        ),
    >,
    mut removed_prims: RemovedComponents<UsdPrimRef>,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut removed_materials: RemovedComponents<MeshMaterial3d<StandardMaterial>>,
    mut mesh_asset_events: Option<MessageReader<AssetEvent<Mesh>>>,
    mut dirty_set: Option<ResMut<RenderProjectionDirtySet>>,
    mesh_consumers: Option<Res<MeshProjectionConsumers>>,
    #[cfg(test)] mut stats: Option<ResMut<SolariProjectionStats>>,
) {
    let Some(meshes) = meshes else {
        return;
    };
    let hardware_available = capability.compiled && capability.device_supported;
    if !hardware_available {
        disable_projection(
            &mut capability,
            &mut state,
            &mut diagnostics,
            &mut commands,
            &mut marked_entities,
        );
        return;
    }

    if let Some(mesh_asset_events) = mesh_asset_events.as_mut() {
        if let (Some(dirty_set), Some(mesh_consumers)) =
            (dirty_set.as_deref_mut(), mesh_consumers.as_deref())
        {
            for event in mesh_asset_events.read() {
                let id = match event {
                    AssetEvent::Added { id }
                    | AssetEvent::LoadedWithDependencies { id }
                    | AssetEvent::Modified { id }
                    | AssetEvent::Removed { id }
                    | AssetEvent::Unused { id } => *id,
                };
                for entity in mesh_consumers.consumers_for(id) {
                    dirty_set.mark(entity);
                }
            }
        } else {
            mesh_asset_events.read().for_each(|_| {});
        }
    }

    let dirty_entities = dirty_set
        .as_deref_mut()
        .map(RenderProjectionDirtySet::take)
        .unwrap_or_default();

    let requested_ray_traced =
        toggles.renderer.render_mode == viewport_protocol::RenderMode::RayTraced;
    if !state.device_supported {
        state.device_supported = true;
        state.initialized = false;
    }

    let full_scan = !state.initialized || (requested_ray_traced && !state.requested_ray_traced);
    if full_scan {
        full_projection_scan(
            &meshes,
            requested_ray_traced,
            &mut capability,
            &mut state,
            &mut diagnostics,
            &mut commands,
            &mut projection_query,
            #[cfg(test)]
            stats.as_deref_mut(),
        );
        return;
    }

    let mut affected = HashSet::new();
    for entity in removed_prims.read().chain(removed_meshes.read()) {
        remove_entry(entity, &mut state, &mut diagnostics, &mut commands);
    }
    for entity in removed_materials.read() {
        if let Ok((entity, mesh, material, marker)) = projection_query.get(entity) {
            update_entry(
                entity,
                mesh,
                material,
                marker,
                &meshes,
                &mut state,
                &mut diagnostics,
                &mut commands,
            );
            affected.insert(entity);
        }
    }
    for entity in dirty_entities {
        if let Ok((entity, mesh, material, marker)) = projection_query.get(entity) {
            update_entry(
                entity,
                mesh,
                material,
                marker,
                &meshes,
                &mut state,
                &mut diagnostics,
                &mut commands,
            );
            affected.insert(entity);
            #[cfg(test)]
            if let Some(stats) = stats.as_deref_mut() {
                stats.incremental_entities += 1;
            }
        }
    }

    set_scene_eligibility(&mut capability, &diagnostics);
    let enabled = requested_ray_traced && capability.supported();
    if enabled != state.enabled {
        for (entity, entry) in &mut state.entries {
            apply_marker(*entity, entry, enabled, &mut commands);
        }
    } else {
        for entity in affected {
            if let Some(entry) = state.entries.get_mut(&entity) {
                apply_marker(entity, entry, enabled, &mut commands);
            }
        }
    }
    if !enabled {
        for entity in &mut marked_entities {
            commands
                .entity(entity)
                .remove::<bevy::solari::prelude::RaytracingMesh3d>();
        }
    }
    if state.requested_ray_traced != requested_ray_traced {
        state.requested_ray_traced = requested_ray_traced;
    }
    if state.enabled != enabled {
        state.enabled = enabled;
    }
}

#[cfg(not(feature = "solari"))]
pub(super) fn sync_solari_usd_projection() {}

#[cfg(feature = "solari")]
fn full_projection_scan(
    meshes: &Assets<Mesh>,
    requested_ray_traced: bool,
    capability: &mut SolariCapability,
    state: &mut SolariProjectionState,
    diagnostics: &mut SolariProjectionDiagnostics,
    commands: &mut Commands,
    query: &mut Query<
        (
            Entity,
            &Mesh3d,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&bevy::solari::prelude::RaytracingMesh3d>,
        ),
        With<UsdPrimRef>,
    >,
    #[cfg(test)] stats: Option<&mut SolariProjectionStats>,
) {
    state.entries.clear();
    *diagnostics = SolariProjectionDiagnostics::default();
    let mut candidates = Vec::new();
    for (entity, mesh, material, marker) in query.iter_mut() {
        let (eligible, missing_material) = evaluate_mesh(mesh, material, meshes);
        diagnostics.candidate_meshes += 1;
        diagnostics.eligible_meshes += u32::from(eligible);
        diagnostics.unsupported_meshes += u32::from(!eligible);
        diagnostics.missing_materials += u32::from(missing_material);
        state.entries.insert(
            entity,
            SolariProjectionEntry {
                mesh: mesh.0.clone(),
                eligible,
                missing_material,
                raytracing_marker: marker.is_some(),
            },
        );
        candidates.push((entity, marker.is_some()));
    }
    set_scene_eligibility(capability, diagnostics);
    let enabled = requested_ray_traced && capability.supported();
    for (entity, marker_present) in candidates {
        if let Some(entry) = state.entries.get_mut(&entity) {
            entry.raytracing_marker = marker_present;
            apply_marker(entity, entry, enabled, commands);
        }
    }
    state.initialized = true;
    state.device_supported = true;
    state.requested_ray_traced = requested_ray_traced;
    state.enabled = enabled;
    #[cfg(test)]
    if let Some(stats) = stats {
        stats.full_scans += 1;
    }
}

#[cfg(feature = "solari")]
fn update_entry(
    entity: Entity,
    mesh: &Mesh3d,
    material: Option<&MeshMaterial3d<StandardMaterial>>,
    marker: Option<&bevy::solari::prelude::RaytracingMesh3d>,
    meshes: &Assets<Mesh>,
    state: &mut SolariProjectionState,
    diagnostics: &mut SolariProjectionDiagnostics,
    commands: &mut Commands,
) {
    if let Some(previous) = state.entries.remove(&entity) {
        adjust_diagnostics(diagnostics, &previous, -1);
        if previous.raytracing_marker && marker.is_some_and(|current| current.0.id() != mesh.0.id())
        {
            commands
                .entity(entity)
                .remove::<bevy::solari::prelude::RaytracingMesh3d>();
        }
    }
    let (eligible, missing_material) = evaluate_mesh(mesh, material, meshes);
    let entry = SolariProjectionEntry {
        mesh: mesh.0.clone(),
        eligible,
        missing_material,
        raytracing_marker: marker.is_some_and(|current| current.0.id() == mesh.0.id()),
    };
    adjust_diagnostics(diagnostics, &entry, 1);
    state.entries.insert(entity, entry);
}

#[cfg(feature = "solari")]
fn remove_entry(
    entity: Entity,
    state: &mut SolariProjectionState,
    diagnostics: &mut SolariProjectionDiagnostics,
    commands: &mut Commands,
) {
    let Some(entry) = state.entries.remove(&entity) else {
        return;
    };
    adjust_diagnostics(diagnostics, &entry, -1);
    if entry.raytracing_marker {
        commands
            .entity(entity)
            .remove::<bevy::solari::prelude::RaytracingMesh3d>();
    }
}
