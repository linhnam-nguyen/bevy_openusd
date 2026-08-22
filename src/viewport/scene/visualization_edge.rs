use std::collections::HashMap;

use bevy::asset::prelude::AssetChanged;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;

use super::DisplayToggles;

/// Cached edge geometry and its shared presentation material.
#[derive(Resource, Debug, Default)]
pub(super) struct EdgeOverlayCache {
    meshes: HashMap<AssetId<Mesh>, Handle<Mesh>>,
    last_enabled: Option<bool>,
}

/// Observable proof that the independent edge pass is enabled and doing work.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EdgeOverlayStats {
    pub enabled: bool,
    pub cached_meshes: u64,
    pub mesh_builds: u64,
}

#[derive(Resource, Debug, Clone)]
pub(super) struct EdgeOverlayMaterial(pub(super) Handle<StandardMaterial>);

/// Marks a child entity as the cached edge pass for one USD mesh entity.
#[derive(Component, Debug)]
pub(crate) struct EdgeOverlay {
    source_mesh: AssetId<Mesh>,
}

pub(super) fn init_edge_overlay_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Option<Res<EdgeOverlayMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    commands.insert_resource(EdgeOverlayMaterial(materials.add(StandardMaterial {
        base_color: Color::srgba(0.12, 0.80, 1.0, 0.9),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        // Metal rejects depth bias for the generated LineList topology.
        depth_bias: 0.0,
        ..default()
    })));
}

/// Synchronizes the independent edge pass only on a renderer-config or mesh
/// change. The source USD entity owns the transform; the child owns only the
/// cached line-list geometry and presentation material.
pub(super) fn sync_edge_overlays(
    toggles: Res<DisplayToggles>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    material: Option<Res<EdgeOverlayMaterial>>,
    mut cache: ResMut<EdgeOverlayCache>,
    mut stats: ResMut<EdgeOverlayStats>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
    mut commands: Commands,
    sources: Query<(Entity, &Mesh3d, Option<&Children>), With<UsdPrimRef>>,
    changed_sources: Query<
        (Entity, &Mesh3d, Option<&Children>),
        (
            With<UsdPrimRef>,
            Or<(Added<Mesh3d>, Changed<Mesh3d>, AssetChanged<Mesh3d>)>,
        ),
    >,
    source_children: Query<&Children, With<UsdPrimRef>>,
    mut overlays: Query<
        (Entity, &mut EdgeOverlay, &mut Mesh3d, &mut Visibility),
        Without<UsdPrimRef>,
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
) {
    let Some(material) = material else {
        return;
    };

    stats.enabled = toggles.renderer.edges;

    for source in removed_meshes.read() {
        let Ok(children) = source_children.get(source) else {
            continue;
        };
        for child in children {
            if let Ok((_, overlay, _, _)) = overlays.get(*child) {
                cache.meshes.remove(&overlay.source_mesh);
                commands.entity(*child).despawn();
            }
        }
    }

    let edge_toggle_changed = cache.last_enabled != Some(toggles.renderer.edges);
    cache.last_enabled = Some(toggles.renderer.edges);

    if edge_toggle_changed {
        for (entity, mesh, children) in &sources {
            sync_one_edge_overlay(
                &mut commands,
                entity,
                mesh,
                children,
                toggles.renderer.edges,
                &mut mesh_assets,
                &material.0,
                &mut cache,
                &mut stats,
                &mut overlays,
            );
        }
    } else {
        for (entity, mesh, children) in &changed_sources {
            sync_one_edge_overlay(
                &mut commands,
                entity,
                mesh,
                children,
                toggles.renderer.edges,
                &mut mesh_assets,
                &material.0,
                &mut cache,
                &mut stats,
                &mut overlays,
            );
        }
    }

    stats.cached_meshes = cache.meshes.len() as u64;
    if let Some(ref mut counters) = counters
        && counters.configuration_edges_enabled != stats.enabled
    {
        counters.configuration_edges_enabled = stats.enabled;
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_one_edge_overlay(
    commands: &mut Commands,
    source: Entity,
    mesh: &Mesh3d,
    children: Option<&Children>,
    enabled: bool,
    mesh_assets: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    cache: &mut EdgeOverlayCache,
    stats: &mut EdgeOverlayStats,
    overlays: &mut Query<
        (Entity, &mut EdgeOverlay, &mut Mesh3d, &mut Visibility),
        Without<UsdPrimRef>,
    >,
) {
    let source_mesh_id = mesh.0.id();
    let existing_child =
        children.and_then(|children| children.iter().find(|child| overlays.get(*child).is_ok()));

    if mesh_assets.get(source_mesh_id).is_none() {
        if let Some(child) = existing_child
            && let Ok((_, _, _, mut visibility)) = overlays.get_mut(child)
        {
            set_edge_visibility(&mut visibility, false);
        }
        return;
    }

    let edge_handle = if enabled {
        edge_mesh_handle(source_mesh_id, mesh_assets, cache, stats)
    } else {
        None
    };

    if let Some(child) = existing_child {
        let Ok((_, mut overlay, mut edge_mesh, mut visibility)) = overlays.get_mut(child) else {
            return;
        };

        if overlay.source_mesh != source_mesh_id {
            cache.meshes.remove(&overlay.source_mesh);
        }
        overlay.source_mesh = source_mesh_id;
        if let Some(edge_handle) = edge_handle {
            edge_mesh.0 = edge_handle;
            set_edge_visibility(&mut visibility, true);
        } else {
            set_edge_visibility(&mut visibility, false);
        }
        return;
    }

    let Some(edge_handle) = edge_handle else {
        return;
    };

    commands.entity(source).with_children(|parent| {
        parent.spawn((
            EdgeOverlay {
                source_mesh: source_mesh_id,
            },
            Mesh3d(edge_handle),
            MeshMaterial3d(material.clone()),
            bevy::pbr::wireframe::NoWireframe,
            if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
    });
}

fn set_edge_visibility(visibility: &mut Visibility, visible: bool) {
    let desired = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != desired {
        *visibility = desired;
    }
}

fn edge_mesh_handle(
    source_mesh_id: AssetId<Mesh>,
    mesh_assets: &mut Assets<Mesh>,
    cache: &mut EdgeOverlayCache,
    stats: &mut EdgeOverlayStats,
) -> Option<Handle<Mesh>> {
    if let Some(handle) = cache.meshes.get(&source_mesh_id) {
        return Some(handle.clone());
    }

    let edge_mesh = {
        let source_mesh = mesh_assets.get(source_mesh_id)?;
        super::edge_mesh::build_edge_mesh(source_mesh)?
    };
    let handle = mesh_assets.add(edge_mesh);
    cache.meshes.insert(source_mesh_id, handle.clone());
    stats.mesh_builds += 1;
    Some(handle)
}
