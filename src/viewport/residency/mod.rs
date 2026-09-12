//! Centralized Scene payload residency for the native viewport.

mod animation;
mod authority;
mod camera;
mod loader;
mod lru;
mod projection;
mod repair;
mod repair_persistence;
mod repair_phase;
mod selection;
mod spatial;
mod viewpoint;
mod worker;

use bevy::asset::Assets;
use bevy::camera::{Camera, CameraProjection, Projection};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetBytesPerFrame;

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::viewport::scene::SectionBoxState;
use crate::viewport::session::SceneCachePresentation;

use animation::{AnimationResidencyState, sync_animation_residency};
use loader::LoadJob;
use projection::SceneResidencyProjection;
use repair::{
    TargetedRepairQueue, drain_cached_residency_completions,
    drain_targeted_repair_persistence_completions, process_targeted_residency_repairs,
};
use repair_persistence::TargetedRepairPersistenceWorker;
use selection::{SelectionResidencyState, release_selected_residency, sync_selected_residency};
use viewpoint::{ActiveViewpointResidencyState, sync_active_viewpoint_residency};
use worker::CachedResidencyWorker;

pub(crate) use authority::{
    DEFAULT_UPLOAD_BYTES_PER_FRAME, PayloadResidencyState, ResidencyAuthority, ResidencyBudgets,
    ResidencyReason, ScenePayloadKey,
};
pub(crate) use camera::{CameraAdmission, CameraSample, SectionBoxClipPlanes};

pub(crate) struct ResidencyPlugin;

impl Plugin for ResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResidencyAuthority>()
            .init_resource::<SceneResidencyProjection>()
            .init_resource::<TargetedRepairQueue>()
            .init_resource::<TargetedRepairPersistenceWorker>()
            .init_resource::<CachedResidencyWorker>()
            .init_resource::<AnimationResidencyState>()
            .init_resource::<ActiveViewpointResidencyState>()
            .init_resource::<SelectionResidencyState>()
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
                    update_camera_residency,
                    drain_cached_residency_completions,
                    drain_targeted_repair_persistence_completions,
                    process_targeted_residency_repairs,
                    dispatch_cached_residency_loads,
                    pump_residency_uploads,
                )
                    .chain(),
            );
    }
}

fn sync_scene_cache_candidates(
    presentation: Option<Res<SceneCachePresentation>>,
    mut authority: ResMut<ResidencyAuthority>,
    mut assets: ResMut<Assets<Mesh>>,
    mut projection: ResMut<SceneResidencyProjection>,
    mut repairs: ResMut<TargetedRepairQueue>,
    mut commands: Commands,
) {
    let Some(presentation) = presentation else {
        authority.retire();
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
    authority.update_camera_near(CameraAdmission {
        sample,
        frustum: projection.compute_frustum(global_transform),
        section_box,
    });
}

fn dispatch_cached_residency_loads(
    cache_context: Option<Res<ActiveProjectCacheContext>>,
    worker: Res<CachedResidencyWorker>,
    mut authority: ResMut<ResidencyAuthority>,
) {
    let Some(cache_context) = cache_context else {
        return;
    };
    if !worker.is_available() {
        return;
    }
    let project_root = cache_context.project_root.clone();
    while let Some(job) = authority.begin_next_load() {
        if let Err(job) = worker.dispatch(project_root.clone(), job) {
            let _ = authority.defer_load(job);
            break;
        }
    }
}

fn pump_residency_uploads(
    mut authority: ResMut<ResidencyAuthority>,
    mut assets: ResMut<Assets<Mesh>>,
    render_budget: Res<RenderAssetBytesPerFrame>,
    mut projection: ResMut<SceneResidencyProjection>,
    mut commands: Commands,
) {
    for key in authority.pump_uploads(&mut assets, render_budget.max_bytes) {
        if let Some(handle) = authority.render_handle(&key) {
            projection.attach_payload(key, handle, &mut commands);
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
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    use usd_model::HashDigest;
    use usd_project::SceneId;

    #[test]
    fn default_budget_matches_bevy_upload_throttle() {
        let budgets = ResidencyBudgets::default();
        assert_eq!(
            budgets.upload_bytes_per_frame,
            DEFAULT_UPLOAD_BYTES_PER_FRAME
        );
    }

    #[test]
    fn lifecycle_retirement_rejects_stale_residency_completion() {
        let scene = SceneId::new_v4();
        let key = ScenePayloadKey {
            scene_id: scene,
            blob_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
        };
        let mut authority = ResidencyAuthority::default();
        authority.install_scene(scene, 41, Vec::new());
        assert!(authority.request_reason(key, ResidencyReason::CameraNear, 41, 8, 8));
        let stale_job = authority.begin_next_load().expect("queued camera payload");

        let mut world = World::new();
        world.insert_resource(authority);
        world.insert_resource(Assets::<Mesh>::default());
        retire_scene_cache_resources(&mut world);

        let mut authority = world.resource_mut::<ResidencyAuthority>();
        assert_eq!(authority.state(&key), None);
        assert_eq!(authority.accounted_bytes(), (0, 0));
        assert!(!authority.complete_cached_cpu(
            stale_job,
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default()
            ),
        ));
    }
}
