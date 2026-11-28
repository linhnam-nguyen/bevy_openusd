use bevy::prelude::*;
use usd_bevy::LiveStage;

use super::{StageHandle, StageInfo, StagePresentationContext, project_activation};
use crate::cache::generic::{
    ActiveStandaloneCacheHandle, CacheIdentity, PendingStandaloneCachePublish, SourceIdentity,
    StandaloneCacheProviderResource, compute_standalone_source_identity,
    source_identity_from_opened_stage,
};
use crate::viewport::stage::SceneCacheOwnershipContext;

pub(super) fn open_stage(world: &mut World, path: std::path::PathBuf) {
    let active = world.get_non_send::<LiveStage>();
    if active.is_some_and(LiveStage::is_authoring_frozen) {
        warn!("cannot replace the active USD stage while authoring is frozen");
        return;
    }
    let has_active_stage = active.is_some();
    let same_document = world
        .get_resource::<StageHandle>()
        .is_some_and(|handle| handle.path == path);
    let (presentation, generation, owner) = if same_document {
        (
            world
                .get_resource::<StagePresentationContext>()
                .cloned()
                .unwrap_or_default(),
            world
                .get_resource::<StageInfo>()
                .map_or(0, |info| info.activation_generation),
            world.get_resource::<SceneCacheOwnershipContext>().cloned(),
        )
    } else {
        (StagePresentationContext::default(), 0, None)
    };
    // Reloading the same document does not submit a new Project activation.
    // Preserve its terminal initial result; ordinary projection readiness still
    // gates the new LiveStage while its bindings are rebuilt.
    let terminal_initial_result = if has_active_stage && same_document && generation != 0 {
        world.get_resource::<StageInfo>().and_then(|info| {
            info.initial_gpu_projection.clone().filter(|status| {
                matches!(
                    status,
                    viewport_protocol::InitialGpuProjection::Complete
                        | viewport_protocol::InitialGpuProjection::Failed { .. }
                )
            })
        })
    } else {
        None
    };
    let standalone_cache_enabled =
        owner.is_none() && world.contains_resource::<StandaloneCacheProviderResource>();
    let pre_open_source = standalone_cache_enabled
        .then(|| compute_standalone_source_identity(&path).ok())
        .flatten();
    let result = LiveStage::open_document(&path)
        .map_err(|error| format!("failed to open {}: {error:#}", path.display()))
        .and_then(|stage| {
            if let Some(active) = world.get_non_send::<LiveStage>()
                && same_document
            {
                reject_loss_of_edits(&active.stage, &stage.stage)
                    .map_err(|error| error.to_string())?;
            }
            let stage_identity = stage.stage_identity();
            let authoring_generation = stage.authoring_generation();
            let bound_source = if standalone_cache_enabled {
                bind_opened_stage_source_identity(&path, &stage, pre_open_source)
            } else {
                None
            };
            project_activation::activate_open_stage_with_cache_context_for_generation(
                world,
                path.clone(),
                stage,
                None,
                None,
                owner.as_ref().map(|owner| owner.project_root.clone()),
                owner.as_ref().map(|owner| owner.scene_id),
                None,
                generation,
                presentation,
                project_activation::StageInstallMode::Fresh,
            )?;
            if let Some(status) = terminal_initial_result
                && let Some(mut info) = world.get_resource_mut::<StageInfo>()
                && info.initial_gpu_projection.is_some()
            {
                info.initial_gpu_projection = Some(status);
            }
            if let Some(owner) = owner {
                world.remove_resource::<ActiveStandaloneCacheHandle>();
                world.remove_resource::<PendingStandaloneCachePublish>();
                world.insert_resource(owner);
            } else {
                hydrate_or_schedule_standalone_cache(
                    world,
                    bound_source,
                    stage_identity,
                    authoring_generation,
                    generation,
                );
            }
            Ok(())
        });
    if let Err(message) = result {
        error!("{message}");
        if !has_active_stage {
            world.insert_resource(StageHandle {
                path,
                error: Some(message),
            });
        }
    }
}

fn reject_loss_of_edits(
    active: &openusd::usd::Stage,
    candidate: &openusd::usd::Stage,
) -> anyhow::Result<()> {
    for identifier in active.layer_identifiers() {
        let Some(layer) = active.layer(&identifier)? else {
            continue;
        };
        if !layer.try_borrow()?.is_dirty() {
            continue;
        }
        // Export-based saves can leave the native dirty bit set. Compare the
        // authored layer data before deciding that replacement would lose edits.
        if let Some(reloaded) = candidate.layer(&identifier)?
            && layer.export_to_string()? == reloaded.export_to_string()?
        {
            continue;
        }
        anyhow::bail!("cannot reload USD while layer {identifier} has unsaved edits");
    }
    Ok(())
}

/// Fail-closed check for any dirty layer on a composed USD stage.
fn stage_has_dirty_layers(stage: &openusd::usd::Stage) -> bool {
    for identifier in stage.layer_identifiers() {
        let Ok(Some(layer)) = stage.layer(&identifier) else {
            return true;
        };
        let Ok(borrowed) = layer.try_borrow() else {
            return true;
        };
        if borrowed.is_dirty() {
            return true;
        }
    }
    false
}

/// Verifies that the opened [`LiveStage`] is unedited, clean, has no manual
/// visual payload overrides, and matches its actual sealed source buffers to
/// the desired and current source identity. Returns `None` on any mismatch so
/// standalone caching fails closed to ordinary source projection.
fn bind_opened_stage_source_identity(
    path: &std::path::Path,
    stage: &LiveStage,
    pre_open_source: Option<SourceIdentity>,
) -> Option<SourceIdentity> {
    let pre_source = pre_open_source?;
    if stage.has_manual_visual_payloads() || stage_has_dirty_layers(&stage.stage) {
        return None;
    }
    let captured = source_identity_from_opened_stage(stage).ok()?;
    let current = compute_standalone_source_identity(path).ok()?;
    (captured == pre_source && captured == current).then_some(captured)
}

/// Optional standalone generic render-mesh cache hydration for non-Project
/// stage openings. On a validated hit, decoded `Mesh` assets are inserted into
/// `usd_bevy::ProjectionSeed` so `LiveStagePlugin` consumes them directly; on a
/// miss, schedules post-projection publication from actual projected meshes.
fn hydrate_or_schedule_standalone_cache(
    world: &mut World,
    bound_source: Option<SourceIdentity>,
    stage_identity: (u64, u64),
    authoring_generation: usd_bevy::LiveRevision,
    generation: u64,
) {
    world.remove_resource::<ActiveStandaloneCacheHandle>();
    world.remove_resource::<PendingStandaloneCachePublish>();
    let Some(provider) = world
        .get_resource::<StandaloneCacheProviderResource>()
        .cloned()
    else {
        return;
    };
    let Some(source_id) = bound_source else {
        return;
    };
    let identity = CacheIdentity::new(source_id, provider.recipe_digest);
    if let Ok(Some((handle, meshes))) = provider.store.hydrate_meshes(&identity)
        && world.contains_resource::<Assets<Mesh>>()
        && world.contains_resource::<usd_bevy::ProjectionSeed>()
    {
        let mesh_handles: Vec<(String, Handle<Mesh>)> = {
            let mut mesh_assets = world.resource_mut::<Assets<Mesh>>();
            meshes
                .into_iter()
                .map(|(prim_path, mesh)| (prim_path, mesh_assets.add(mesh)))
                .collect()
        };
        let mut seed = world.resource_mut::<usd_bevy::ProjectionSeed>();
        for (prim_path, mesh_handle) in mesh_handles {
            seed.insert_mesh(prim_path, mesh_handle, None);
        }
        world.insert_resource(ActiveStandaloneCacheHandle(handle));
        return;
    }
    world.insert_resource(PendingStandaloneCachePublish {
        identity,
        stage_identity,
        authoring_generation,
        generation,
    });
}

/// Publishes actual projected base `Mesh` payloads into `StandaloneCacheStore`
/// once `LiveStagePlugin` has projected geometry for the matching unedited
/// standalone stage identity, authoring generation, and activation generation.
/// Revalidates stage cleanliness, `opened_root_bytes`, and source closure
/// identity before publication so stale, edited, or mutated sources are never
/// cached.
pub(crate) fn publish_standalone_cache_when_ready(world: &mut World) {
    let Some(pending) = world
        .get_resource::<PendingStandaloneCachePublish>()
        .cloned()
    else {
        return;
    };
    let Some(provider) = world
        .get_resource::<StandaloneCacheProviderResource>()
        .cloned()
    else {
        world.remove_resource::<PendingStandaloneCachePublish>();
        return;
    };
    if world.contains_resource::<SceneCacheOwnershipContext>() {
        world.remove_resource::<PendingStandaloneCachePublish>();
        return;
    }
    let stage_eligible = world
        .get_non_send::<LiveStage>()
        .filter(|stage| {
            stage.stage_identity() == pending.stage_identity
                && stage.authoring_generation() == pending.authoring_generation
                && !stage.has_manual_visual_payloads()
                && !stage_has_dirty_layers(&stage.stage)
        })
        .and_then(|stage| source_identity_from_opened_stage(stage).ok());
    let gen_matches = world
        .get_resource::<StageInfo>()
        .is_some_and(|info| info.activation_generation == pending.generation);
    let Some(_captured) =
        stage_eligible.filter(|source| gen_matches && *source == pending.identity.source)
    else {
        world.remove_resource::<PendingStandaloneCachePublish>();
        return;
    };
    if !world.get_resource::<super::Spawned>().is_some_and(|s| s.0) {
        return;
    }
    let Some(prim_entities) = world.get_resource::<usd_bevy::PrimEntities>() else {
        return;
    };
    let Some(path_store) = world.get_resource::<usd_bevy::PathStore>() else {
        return;
    };
    let mut prim_mesh_handles: Vec<(String, Handle<Mesh>)> = Vec::new();
    for (path, entity) in prim_entities.iter(path_store) {
        if let Some(mesh3d) = world.get::<Mesh3d>(entity) {
            prim_mesh_handles.push((path.to_owned(), mesh3d.0.clone()));
        }
    }
    if prim_mesh_handles.is_empty() {
        return;
    }
    let Some(mesh_assets) = world.get_resource::<Assets<Mesh>>() else {
        return;
    };
    let mut owned_meshes: Vec<(String, Mesh)> = Vec::new();
    for (prim_path, handle) in prim_mesh_handles {
        if let Some(mesh) = mesh_assets.get(&handle) {
            owned_meshes.push((prim_path, mesh.clone()));
        }
    }
    if owned_meshes.is_empty() {
        return;
    }
    world.remove_resource::<PendingStandaloneCachePublish>();
    // The projected meshes are tied to the opened sealed closure above.
    // Revalidate the current source before exposing them as a current cache.
    let Ok(current_source) =
        compute_standalone_source_identity(&pending.identity.source.entry_path)
    else {
        return;
    };
    if current_source != pending.identity.source {
        return;
    }
    let refs: Vec<(String, &Mesh)> = owned_meshes
        .iter()
        .map(|(path, mesh)| (path.clone(), mesh))
        .collect();
    if let Ok(handle) = provider.store.publish_meshes(&pending.identity, &refs) {
        world.insert_resource(ActiveStandaloneCacheHandle(handle));
    }
}
