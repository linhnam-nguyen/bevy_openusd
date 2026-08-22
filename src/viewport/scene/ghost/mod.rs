//! Historical geometry hydration for semantic diff overlays.
//!
//! Ghosts are ordinary Bevy render entities with a dedicated marker. They do
//! not carry `UsdPrimRef`, `UsdEntityKey`, authoring components, or authored
//! materials, so normal scene selection and mutation paths cannot target them.

use std::collections::{HashMap, HashSet};

use bevy::asset::{Assets, Handle};
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use usd_model::{BlobId, EntityKey, SnapshotId};

use super::diff::historical_world_matrix;
use crate::project::blob_store::{FilesystemBlobStore, get_mesh};
use crate::project::ghost_cache::HistoricalGeometryCache;
use crate::project::recovery::RecoverySettings;
use crate::viewport::semantic::SemanticDiffState;

const OBJECTS_DIRECTORY: &str = ".usdhub/cache/objects";
const GHOST_COLOR: Color = Color::srgba(1.0, 0.16, 0.36, 0.38);

/// Marker for historical geometry that is never a normal authoring target.
#[derive(Component, Clone, Debug)]
pub(crate) struct HistoricalGhost {
    pub(crate) entity_key: EntityKey,
    pub(crate) source_snapshot: SnapshotId,
    pub(crate) blob_id: BlobId,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct GhostKey {
    entity_key: EntityKey,
    blob_id: BlobId,
}

#[derive(Clone, Debug)]
struct GhostDescriptor {
    key: GhostKey,
    entity_key: EntityKey,
    source_snapshot: SnapshotId,
    transform: Transform,
}

/// In-memory hydration and entity reuse state for historical ghosts.
#[derive(Default, Resource)]
pub(crate) struct HistoricalGhostState {
    ghost_entities: HashMap<GhostKey, Entity>,
    mesh_handles: HashMap<BlobId, Handle<Mesh>>,
    failed_blobs: HashSet<BlobId>,
    material: Option<Handle<StandardMaterial>>,
}

/// Hydrate old/deleted geometry from persistent render blobs.
///
/// The system only considers removed entities and path-moved entities. Missing
/// or invalid blobs are recorded and left to the existing gizmo fallback in
/// `draw_semantic_diff`.
pub(crate) fn hydrate_historical_ghosts(
    diff: Res<SemanticDiffState>,
    settings: Option<Res<RecoverySettings>>,
    mut state: ResMut<HistoricalGhostState>,
    mut geometry_cache: Option<ResMut<HistoricalGeometryCache>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    current_prims: Query<(&UsdPrimRef, &GlobalTransform)>,
    existing_ghosts: Query<(Entity, &HistoricalGhost)>,
    mut commands: Commands,
) {
    let Some(stage_diff) = diff.stage_diff() else {
        despawn_all_ghosts(&mut commands, &mut state, &existing_ghosts);
        return;
    };
    let Some(settings) = settings else {
        return;
    };
    let Some(mut meshes) = meshes else {
        return;
    };
    let Some(mut materials) = materials else {
        return;
    };

    let root_matrix = current_prims
        .iter()
        .find(|(prim, _)| prim.path == "/")
        .map(|(_, global)| global.compute_transform())
        .map(|transform| {
            Mat4::from_scale_rotation_translation(
                transform.scale,
                transform.rotation,
                transform.translation,
            )
        })
        .unwrap_or(Mat4::IDENTITY);
    let source_snapshot = diff
        .baseline_snapshot_id()
        .expect("a StageDiff always has a baseline snapshot")
        .clone();

    let mut desired = HashMap::new();
    for entity in stage_diff.entities.values() {
        let historical = entity.presence == usd_model::PresenceState::Removed
            || (entity.presence == usd_model::PresenceState::Existing
                && entity.flags.contains(usd_model::ChangeFlags::PATH));
        if !historical {
            continue;
        }
        let Some(old) = entity.old.as_ref() else {
            continue;
        };
        let Some(blob_id) = old
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.render_blob.as_ref())
        else {
            continue;
        };
        let key = GhostKey {
            entity_key: old.key.clone(),
            blob_id: blob_id.clone(),
        };
        desired.insert(
            key.clone(),
            GhostDescriptor {
                key,
                entity_key: old.key.clone(),
                source_snapshot: source_snapshot.clone(),
                transform: Transform::from_matrix(historical_world_matrix(
                    stage_diff,
                    old,
                    root_matrix,
                )),
            },
        );
    }

    let store = match FilesystemBlobStore::new(settings.project_root.join(OBJECTS_DIRECTORY)) {
        Ok(store) => store,
        Err(error) => {
            bevy::log::error!("[ghost-cache] cannot create historical mesh store: {error:#}");
            return;
        }
    };

    let mut hydration_count = 0;
    let mut failure_count = 0;
    let mut descriptors = desired.values().cloned().collect::<Vec<_>>();
    descriptors.sort_by(|left, right| {
        left.key
            .entity_key
            .cmp(&right.key.entity_key)
            .then_with(|| left.key.blob_id.cmp(&right.key.blob_id))
    });

    let mut material_handle = state.material.clone();
    for descriptor in descriptors {
        let blob_id = &descriptor.key.blob_id;
        let mesh_handle = if let Some(handle) = state.mesh_handles.get(blob_id) {
            handle.clone()
        } else if state.failed_blobs.contains(blob_id) {
            continue;
        } else {
            match get_mesh(&store, blob_id) {
                Ok(Some(mesh)) => {
                    let handle = meshes.add(mesh);
                    state.mesh_handles.insert(blob_id.clone(), handle.clone());
                    hydration_count += 1;
                    handle
                }
                Ok(None) => {
                    failure_count += 1;
                    state.failed_blobs.insert(blob_id.clone());
                    bevy::log::warn!(
                        "[ghost-cache] historical mesh blob {} is unavailable",
                        blob_id.0
                    );
                    continue;
                }
                Err(error) => {
                    failure_count += 1;
                    state.failed_blobs.insert(blob_id.clone());
                    bevy::log::warn!(
                        "[ghost-cache] historical mesh blob {} failed to decode: {error:#}",
                        blob_id.0
                    );
                    continue;
                }
            }
        };

        let Some(material) = material_handle.clone().or_else(|| {
            let material = StandardMaterial {
                base_color: GHOST_COLOR,
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..Default::default()
            };
            let handle = materials.add(material);
            material_handle = Some(handle.clone());
            state.material = Some(handle.clone());
            Some(handle)
        }) else {
            continue;
        };

        let existing = existing_ghosts.iter().find_map(|(entity, ghost)| {
            (ghost.entity_key == descriptor.entity_key
                && ghost.blob_id == *blob_id
                && ghost.source_snapshot == descriptor.source_snapshot)
                .then_some(entity)
        });
        let entity = state
            .ghost_entities
            .get(&descriptor.key)
            .copied()
            .filter(|entity| {
                existing_ghosts
                    .iter()
                    .any(|(current, _)| current == *entity)
            })
            .or(existing);
        let ghost = HistoricalGhost {
            entity_key: descriptor.entity_key.clone(),
            source_snapshot: descriptor.source_snapshot.clone(),
            blob_id: blob_id.clone(),
        };
        if let Some(entity) = entity {
            commands.entity(entity).insert((
                ghost,
                Mesh3d(mesh_handle),
                MeshMaterial3d(material),
                descriptor.transform,
                Visibility::Visible,
            ));
            state.ghost_entities.insert(descriptor.key, entity);
        } else {
            let entity = commands
                .spawn((
                    ghost,
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material),
                    descriptor.transform,
                    Visibility::Visible,
                ))
                .id();
            state.ghost_entities.insert(descriptor.key, entity);
        }
    }

    let desired_keys: HashSet<_> = desired.keys().cloned().collect();
    for (entity, ghost) in existing_ghosts.iter() {
        let key = GhostKey {
            entity_key: ghost.entity_key.clone(),
            blob_id: ghost.blob_id.clone(),
        };
        if !desired_keys.contains(&key) {
            commands.entity(entity).despawn();
        }
    }
    // Keep newly queued entities in the registry as well as entities already
    // visible to this frame's query. If an externally removed entity remains
    // here, the next pass will fail the existence filter above and replace it.
    state
        .ghost_entities
        .retain(|key, _| desired_keys.contains(key));

    if let Some(mut cache) = geometry_cache.take() {
        cache.ghost_mesh_hydrations += hydration_count;
        cache.ghost_load_failures += failure_count;
    }
}

fn despawn_all_ghosts(
    commands: &mut Commands,
    state: &mut HistoricalGhostState,
    existing_ghosts: &Query<(Entity, &HistoricalGhost)>,
) {
    for (entity, _) in existing_ghosts.iter() {
        commands.entity(entity).despawn();
    }
    state.ghost_entities.clear();
}

#[cfg(test)]
mod tests;
