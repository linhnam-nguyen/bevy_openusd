//! Centralized Scene payload residency for the native viewport.

mod animation;
mod authority;
mod camera;
mod catalog;
mod generated_lod;
pub(crate) use generated_lod::GeneratedLodSource;
mod loader;
mod lru;
mod projection;
mod repair;
mod selection;
mod spatial;
mod viewpoint;
mod visual_composition;
pub(crate) use visual_composition::{EditPayloadPins, VisualStreamingHealth};
mod visual_lod;
mod visual_policy;
mod worker;

use bevy::asset::Assets;
use bevy::camera::{Camera, CameraProjection, Projection};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetBytesPerFrame;

use crate::cache::contract::{SceneCacheAddress, SceneCacheOccurrence};
use crate::cache::hydration::ActiveProjectCacheContext;
use crate::viewport::ViewportFrameSet;
use crate::viewport::rendering::scene::{SceneAnchorIndex, SectionBoxState};
use crate::viewport::stage::{SceneCacheOwnershipContext, SceneCachePresentation};

use animation::{AnimationResidencyState, sync_animation_residency};
use loader::LoadJob;
pub(crate) use projection::SceneResidencyProjection;
use repair::{
    TargetedRepairPersistenceWorker, TargetedRepairQueue, drain_cached_residency_completions,
    drain_targeted_repair_persistence_completions, process_targeted_residency_repairs,
};
use selection::{SelectionResidencyState, release_selected_residency, sync_selected_residency};
use usd_bevy::live::source_mesh::{SourceMeshIdentity, SourceMeshInput, publish_source_mesh};
use usd_bevy::route::geom::{DeferHiddenSourceMeshes, DeferredSourceMesh, source_mesh_is_hidden};
use viewpoint::{ActiveViewpointResidencyState, sync_active_viewpoint_residency};
use worker::{CachedResidencyWorker, LoadedScenePayloadQueue, install_loaded_scene_payloads};

pub(crate) use authority::{
    DEFAULT_UPLOAD_BYTES_PER_FRAME, PayloadResidencyState, ResidencyAuthority, ResidencyBudgets,
    ResidencyReason, ScenePayloadKey,
};
pub(crate) use camera::{CameraAdmission, CameraSample, SectionBoxClipPlanes};
pub(crate) use catalog::{PayloadLoadMask, ScenePayloadCatalog, ScenePayloadDescriptor};

pub(crate) struct ResidencyPlugin;

impl Plugin for ResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResidencyAuthority>()
            .init_resource::<visual_composition::VisualStreaming>()
            .init_resource::<visual_composition::EditPayloadPins>()
            .init_resource::<visual_composition::VisualStreamingHealth>()
            .init_resource::<ScenePayloadCatalog>()
            .init_resource::<SceneResidencyProjection>()
            .init_resource::<TargetedRepairQueue>()
            .init_resource::<TargetedRepairPersistenceWorker>()
            .init_resource::<CachedResidencyWorker>()
            .init_resource::<LoadedScenePayloadQueue>()
            .init_resource::<usd_bevy::route::material::UsdTextureCache>()
            .init_resource::<AnimationResidencyState>()
            .init_resource::<ActiveViewpointResidencyState>()
            .init_resource::<SelectionResidencyState>()
            .init_resource::<DeferHiddenSourceMeshes>()
            .init_resource::<SourceDemandScan>()
            .init_resource::<VisibilityChangeEpoch>()
            .insert_resource(RenderAssetBytesPerFrame::new(
                DEFAULT_UPLOAD_BYTES_PER_FRAME,
            ))
            .add_systems(
                Update,
                (
                    sync_scene_cache_candidates,
                    sync_animation_residency,
                    sync_active_viewpoint_residency,
                    sync_selected_residency,
                    advance_visibility_epoch
                        .after(ViewportFrameSet::ApplyTreeCommands)
                        .after(usd_bevy::LiveStageSet::Presentation),
                    update_camera_residency,
                    visual_composition::synchronize_visual_composition,
                    visual_lod::synchronize_authored_lod
                        .before(ViewportFrameSet::PublishStageLoadState),
                    drain_cached_residency_completions,
                    install_loaded_scene_payloads,
                    drain_targeted_repair_persistence_completions,
                    process_targeted_residency_repairs,
                    dispatch_source_meshes,
                    dispatch_cached_residency_loads,
                    pump_residency_uploads,
                    generated_lod::synchronize_generated_lod,
                )
                    .chain(),
            );
    }
}

fn visual_projection_settled(state: Option<&usd_bevy::ProgressiveProjectionState>) -> bool {
    state.is_none_or(|state| {
        matches!(
            state.readiness(),
            usd_bevy::ProjectionReadiness::Idle | usd_bevy::ProjectionReadiness::Ready
        )
    })
}

#[derive(Component)]
struct PreparingSourceMesh(SourceMeshIdentity);

#[derive(Resource, Default)]
struct SourceDemandScan(Option<SourceDemandStamp>);

#[derive(Resource, Default)]
struct VisibilityChangeEpoch(u64);

fn advance_visibility_epoch(
    changed: Query<
        (),
        (
            Without<projection::SceneResidencyOccurrence>,
            Or<(Changed<Visibility>, Changed<ChildOf>)>,
        ),
    >,
    mut removed_visibility: RemovedComponents<Visibility>,
    mut removed_parent: RemovedComponents<ChildOf>,
    mut epoch: ResMut<VisibilityChangeEpoch>,
) {
    let removed_visibility = removed_visibility.read().count() != 0;
    let removed_parent = removed_parent.read().count() != 0;
    if removed_visibility || removed_parent || !changed.is_empty() {
        epoch.0 = epoch.0.saturating_add(1);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceDemandStamp {
    context: usd_bevy::VisualContextId,
    generation: u64,
    scene_revision: u64,
    selection_revision: u64,
    visibility_epoch: u64,
    enabled: bool,
}

fn source_mesh_requested(world: &World, entity: Entity) -> bool {
    if !world.contains_resource::<DeferHiddenSourceMeshes>()
        || !source_mesh_is_hidden(world, entity)
    {
        return true;
    }
    world
        .get_resource::<SceneAnchorIndex>()
        .and_then(|index| index.anchor_for(entity))
        .is_some_and(|anchor| {
            world
                .get_resource::<crate::viewport::rendering::scene::SelectedTargets>()
                .is_some_and(|selected| selected.0.targets.contains(&anchor))
        })
}

fn dispatch_source_meshes(world: &mut World) {
    let stamp = world
        .get_non_send::<usd_bevy::LiveStage>()
        .and_then(|live| {
            let projection = world.get_resource::<usd_bevy::ProgressiveProjectionState>()?;
            if !projection.native_instance_dependencies_ready()
                || projection.session_id() != Some(live.session_id())
                || live.has_changes()
            {
                return None;
            }
            let scene = world.get_resource::<SceneAnchorIndex>()?;
            let selection = world.get_resource::<crate::viewport::rendering::scene::SelectedTargets>()?;
            Some(SourceDemandStamp {
                context: live.visual_context_id(),
                generation: projection.generation(),
                scene_revision: scene.revision(),
                selection_revision: selection.revision(),
                visibility_epoch: world.get_resource::<VisibilityChangeEpoch>()?.0,
                enabled: world.contains_resource::<DeferHiddenSourceMeshes>(),
            })
        });
    if stamp.is_none()
        && let Some(mut scan) = world.get_resource_mut::<SourceDemandScan>()
        && scan.0.is_some()
    {
        scan.0 = None;
    }
    if stamp.is_some()
        && world
            .get_resource::<SourceDemandScan>()
            .is_some_and(|scan| scan.0 == stamp)
    {
        return;
    }
    let mut deferred =
        world.query_filtered::<Entity, (With<DeferredSourceMesh>, Without<PreparingSourceMesh>)>();
    let requested = deferred
        .iter(world)
        .filter(|&entity| source_mesh_requested(world, entity))
        .take(worker::WORKER_QUEUE_CAPACITY)
        .collect::<Vec<_>>();
    if requested.is_empty() {
        if let Some(mut scan) = world.get_resource_mut::<SourceDemandScan>() {
            scan.0 = stamp;
        }
        return;
    }
    let worker = world.resource::<CachedResidencyWorker>();
    if !worker.is_available() {
        if let Some(identity) = requested
            .into_iter()
            .find_map(|entity| SourceMeshIdentity::capture(world, entity))
        {
            publish_source_mesh(world, identity, None);
        }
        return;
    }
    let Some(capacity) = worker.source_capacity() else {
        return;
    };
    let mut batch = Vec::with_capacity(capacity.jobs);
    let mut remaining_bytes = capacity.bytes;
    for entity in requested.into_iter().take(capacity.jobs) {
        let Some(identity) = SourceMeshIdentity::capture(world, entity) else {
            continue;
        };
        let Some(input) = SourceMeshInput::capture(world, identity) else {
            if batch.is_empty() {
                publish_source_mesh(world, identity, None);
            }
            break;
        };
        let bytes = input.estimated_bytes();
        if bytes > remaining_bytes {
            if batch.is_empty() && capacity.allow_oversized {
                batch.push((bytes, input));
            }
            break;
        }
        batch.push((bytes, input));
        remaining_bytes -= bytes;
        if remaining_bytes == 0 {
            break;
        }
    }
    batch.sort_by_key(|(bytes, _)| std::cmp::Reverse(*bytes));
    for (_, input) in batch {
        let identity = input.identity;
        if !world
            .resource::<CachedResidencyWorker>()
            .dispatch_source(input)
        {
            break;
        }
        world
            .entity_mut(identity.entity)
            .insert(PreparingSourceMesh(identity));
    }
}

fn complete_source_mesh(world: &mut World, identity: SourceMeshIdentity, mesh: Option<Mesh>) {
    if !world
        .get::<PreparingSourceMesh>(identity.entity)
        .is_some_and(|pending| pending.0 == identity)
    {
        return;
    }
    world
        .entity_mut(identity.entity)
        .remove::<PreparingSourceMesh>();
    if source_mesh_requested(world, identity.entity) {
        if let Some(mut scan) = world.get_resource_mut::<SourceDemandScan>() {
            scan.0 = None;
        }
        publish_source_mesh(world, identity, mesh);
    }
}

fn sync_scene_cache_candidates(
    presentation: Option<Res<SceneCachePresentation>>,
    mut authority: ResMut<ResidencyAuthority>,
    mut assets: ResMut<Assets<Mesh>>,
    mut projection: ResMut<SceneResidencyProjection>,
    mut catalog: ResMut<ScenePayloadCatalog>,
    mut repairs: ResMut<TargetedRepairQueue>,
    mut commands: Commands,
) {
    let Some(presentation) = presentation else {
        authority.retire();
        *catalog = ScenePayloadCatalog::default();
        repairs.clear();
        projection.retire(&mut commands);
        release_retired_render_assets(&mut authority, &mut assets, &mut projection, &mut commands);
        return;
    };
    if !presentation.is_changed() {
        return;
    }
    repairs.clear();
    let payloads = spatial::scene_payloads(&presentation.entries);
    match ScenePayloadCatalog::from_presentation(&presentation) {
        Ok(next) => *catalog = next,
        Err(error) => {
            bevy::log::warn!(
                "[viewport-residency] rejected invalid Scene payload catalog: {error}"
            );
            *catalog = ScenePayloadCatalog::default();
        }
    }
    authority.install_scene(
        presentation.scene_id,
        presentation.generation,
        payloads.clone(),
    );
    projection.retire(&mut commands);
    projection.install_scene(&payloads);
    release_retired_render_assets(&mut authority, &mut assets, &mut projection, &mut commands);
}

fn update_camera_residency(
    mut authority: ResMut<ResidencyAuthority>,
    section_box: Option<Res<SectionBoxState>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    scene_index: Option<Res<SceneAnchorIndex>>,
    epoch: Res<VisibilityChangeEpoch>,
    visibility: Query<(Option<&Visibility>, Option<&ChildOf>)>,
) {
    let Ok((camera, global_transform, projection)) = cameras.single() else {
        return;
    };
    let transform = global_transform.compute_transform();
    let section_box_revision = section_box.as_ref().map_or(0, |state| state.revision);
    let sample = CameraSample {
        position: transform.translation.to_array().map(f64::from),
        forward: transform.forward().as_vec3().to_array().map(f64::from),
        projection: projection.get_clip_from_view().to_cols_array(),
        viewport_size: camera
            .physical_viewport_size()
            .map_or([0, 0], |size| [size.x, size.y]),
        section_box_revision,
        visibility_revision: scene_index.as_ref().map_or(0, |index| index.revision()),
        visibility_epoch: epoch.0,
        ..CameraSample::default()
    };
    let section_box = section_box.as_deref().and_then(|state| {
        (state.enabled && state.visible).then(|| SectionBoxClipPlanes {
            planes: state
                .clip_planes
                .planes
                .map(|plane| plane.to_array().map(f64::from)),
        })
    });
    authority.update_camera_near_filtered(
        CameraAdmission {
            sample,
            frustum: projection.compute_frustum(global_transform),
            section_box,
        },
        |candidate| {
            cached_occurrence_visible(&candidate.address, scene_index.as_deref(), &visibility)
        },
    );
}

fn cached_occurrence_visible(
    address: &SceneCacheAddress,
    scene_index: Option<&SceneAnchorIndex>,
    visibility: &Query<(Option<&Visibility>, Option<&ChildOf>)>,
) -> bool {
    let (Some(index), SceneCacheOccurrence::PrimPath(path)) = (scene_index, &address.occurrence)
    else {
        return true;
    };
    let occurrences = index.resolve_all_by_prim_path(path);
    occurrences.is_empty()
        || occurrences
            .iter()
            .any(|&entity| occurrence_visible(entity, visibility))
}

fn occurrence_visible(
    mut entity: Entity,
    visibility: &Query<(Option<&Visibility>, Option<&ChildOf>)>,
) -> bool {
    while let Ok((current, parent)) = visibility.get(entity) {
        match (current.copied().unwrap_or(Visibility::Inherited), parent) {
            (Visibility::Hidden, _) => return false,
            (Visibility::Visible, _) | (Visibility::Inherited, None) => return true,
            (Visibility::Inherited, Some(parent)) => entity = parent.parent(),
        }
    }
    true
}

fn dispatch_cached_residency_loads(
    scene_owner: Option<Res<SceneCacheOwnershipContext>>,
    legacy_cache: Option<Res<ActiveProjectCacheContext>>,
    catalog: Option<Res<ScenePayloadCatalog>>,
    worker: Res<CachedResidencyWorker>,
    mut authority: ResMut<ResidencyAuthority>,
) {
    let Some(project_root) =
        active_cache_project_root(scene_owner.as_deref(), legacy_cache.as_deref())
    else {
        return;
    };
    if !worker.is_available() {
        return;
    }
    authority.retry_pending_loads();
    while let Some(job) = authority.begin_next_load() {
        let mask = authority.load_mask(&job.key);
        let descriptors = catalog.as_deref().map_or_else(Vec::new, |catalog| {
            catalog.descriptors_for(job.key).to_vec()
        });
        if let Err(job) =
            worker.dispatch_with_payloads(project_root.clone(), job, mask, descriptors)
        {
            let _ = authority.defer_load(job);
            break;
        }
    }
}

pub(crate) fn active_cache_project_root(
    scene_owner: Option<&SceneCacheOwnershipContext>,
    legacy_cache: Option<&ActiveProjectCacheContext>,
) -> Option<std::path::PathBuf> {
    scene_owner
        .map(|owner| owner.project_root.clone())
        .or_else(|| legacy_cache.map(|context| context.project_root.clone()))
}

fn pump_residency_uploads(
    mut authority: ResMut<ResidencyAuthority>,
    mut assets: ResMut<Assets<Mesh>>,
    render_budget: Res<RenderAssetBytesPerFrame>,
    mut projection: ResMut<SceneResidencyProjection>,
    mut commands: Commands,
    scene_index: Option<Res<SceneAnchorIndex>>,
    epoch: Res<VisibilityChangeEpoch>,
    visibility: Query<(Option<&Visibility>, Option<&ChildOf>)>,
    mut last_visibility_revision: Local<Option<(Option<u64>, u64)>>,
) {
    let is_visible = |address: &SceneCacheAddress| {
        cached_occurrence_visible(address, scene_index.as_deref(), &visibility)
    };
    let visibility_revision = (scene_index.as_ref().map(|index| index.revision()), epoch.0);
    if *last_visibility_revision != Some(visibility_revision) {
        projection.sync_visibility(&mut commands, &is_visible);
        *last_visibility_revision = Some(visibility_revision);
    }
    for key in authority.pump_uploads(&mut assets, render_budget.max_bytes) {
        if let Some(handle) = authority.render_handle(&key) {
            projection.attach_payload(key, handle, &mut commands, &is_visible);
        }
    }
    release_retired_render_assets(&mut authority, &mut assets, &mut projection, &mut commands);
}

fn release_retired_render_assets(
    authority: &mut ResidencyAuthority,
    assets: &mut Assets<Mesh>,
    projection: &mut SceneResidencyProjection,
    commands: &mut Commands,
) {
    for asset_id in authority.take_released_render_assets() {
        projection.release_asset(asset_id, commands);
        let _ = assets.remove(asset_id);
    }
}

pub(crate) fn retire_scene_cache_resources(world: &mut World) {
    generated_lod::clear_generated_lods(world);
    if let Some(mut index) = world.get_resource_mut::<SceneAnchorIndex>() {
        index.clear_visual_unload_rows();
    }
    world.remove_resource::<crate::cache::scene::hydration::SceneAnimationPayloads>();
    if let Some(mut catalog) = world.get_resource_mut::<ScenePayloadCatalog>() {
        *catalog = ScenePayloadCatalog::default();
    }
    release_selected_residency(world);
    let released = world
        .get_resource_mut::<ResidencyAuthority>()
        .map(|mut authority| {
            authority.retire();
            authority.take_released_render_assets()
        })
        .unwrap_or_default();
    if let Some(mut projection) = world.remove_resource::<SceneResidencyProjection>() {
        projection.retire_world(world);
        world.insert_resource(projection);
    }
    if let Some(mut repairs) = world.get_resource_mut::<TargetedRepairQueue>() {
        repairs.clear();
    }
    if let Some(mut assets) = world.get_resource_mut::<Assets<Mesh>>() {
        for asset_id in released {
            let _ = assets.remove(asset_id);
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/residency/mod_inline_tests.rs"]
mod tests;
