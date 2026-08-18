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
            let mut material = StandardMaterial::default();
            material.base_color = GHOST_COLOR;
            material.alpha_mode = AlphaMode::Blend;
            material.unlit = true;
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
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    use tempfile::tempdir;
    use usd_model::{
        Bounds3, EntitySnapshot, GeometrySignature, HashDigest, IdentitySource, QuantizedPoint3,
        SemanticInfo, SemanticSnapshot, SnapshotSource, TransformSignature,
    };

    use super::*;
    use crate::project::blob_store::put_mesh;

    fn digest() -> HashDigest {
        HashDigest::new([9; HashDigest::BYTE_LEN])
    }

    fn mesh() -> Mesh {
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

    fn snapshot(blob_id: Option<BlobId>, path: &str) -> SemanticSnapshot {
        let key = EntityKey::from(path);
        let entity = EntitySnapshot {
            key: key.clone(),
            prim_path: path.to_owned(),
            identity_source: IdentitySource::PrimPath,
            semantic: SemanticInfo::default(),
            transform: TransformSignature {
                translation_mm: [1_000, 0, 0],
                rotation_quantized: [0, 0, 0, 0],
                scale_quantized: [10_000; 3],
                hash: digest(),
            },
            geometry: Some(GeometrySignature {
                vertex_count: 3,
                index_count: 3,
                local_bounds: Bounds3 {
                    min: [0.0; 3],
                    max: [1.0; 3],
                },
                local_centroid: QuantizedPoint3([500; 3]),
                topology_hash: digest(),
                shape_hash: digest(),
                render_blob: blob_id,
            }),
            properties: Vec::new(),
            metadata_hash: digest(),
            full_hash: digest(),
        };
        SemanticSnapshot {
            snapshot_id: SnapshotId(path.to_owned()),
            source: SnapshotSource::Working {
                session: "ghost-test".to_owned(),
                live_revision: 1,
            },
            config_hash: digest(),
            entities: [(key, entity)].into_iter().collect(),
        }
    }

    #[test]
    fn historical_ghost_is_hydrated_reused_and_removed_with_the_diff() -> anyhow::Result<()> {
        let project = tempdir()?;
        let store = FilesystemBlobStore::new(project.path().join(OBJECTS_DIRECTORY))?;
        let blob_id = put_mesh(&store, &mesh())?;

        let baseline = snapshot(Some(blob_id.clone()), "/World/Triangle");
        let working = SemanticSnapshot {
            snapshot_id: SnapshotId("working".to_owned()),
            entities: Default::default(),
            ..baseline.clone()
        };
        let mut diff = SemanticDiffState::default();
        diff.update_working(1, baseline);
        assert!(diff.capture_baseline());
        diff.update_working(1, working);

        let mut app = App::new();
        app.insert_resource(diff)
            .insert_resource(RecoverySettings {
                project_root: project.path().to_path_buf(),
            })
            .insert_resource(HistoricalGeometryCache::default())
            .init_resource::<HistoricalGhostState>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_systems(Update, hydrate_historical_ghosts);

        app.update();
        let first = {
            let mut query = app
                .world_mut()
                .query::<(&HistoricalGhost, &Mesh3d, &Transform)>();
            query
                .iter(app.world())
                .map(|(ghost, _, _)| ghost.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].blob_id, blob_id);
        assert_eq!(
            app.world()
                .resource::<HistoricalGeometryCache>()
                .ghost_mesh_hydrations,
            1
        );

        app.update();
        let second_count = {
            let mut query = app.world_mut().query::<&HistoricalGhost>();
            query.iter(app.world()).count()
        };
        assert_eq!(second_count, 1);

        app.world_mut()
            .resource_mut::<SemanticDiffState>()
            .clear_baseline();
        app.update();
        let final_count = {
            let mut query = app.world_mut().query::<&HistoricalGhost>();
            query.iter(app.world()).count()
        };
        assert_eq!(final_count, 0);
        Ok(())
    }
}
