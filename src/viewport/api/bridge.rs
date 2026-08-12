use bevy::asset::{AssetServer, Assets, LoadState};
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdAsset;
use viewport_protocol::{
    CameraSource, CurveTuning as ProtocolCurveTuning, FocusMode, OverlayKind, PROTOCOL_VERSION,
    PresentationReadModel, SceneAnchor, SelectionReadModel, StageLoadState, StageReadModel,
    TimelineReadModel, ViewportCommand, ViewportEvent, ViewportEventEnvelope, ViewportReadModel,
};

use super::{
    SceneAnchorIndex, SceneQueryService, ViewportCommandInbox, ViewportEventOutbox,
    ViewportTreeCommand, ViewportTreeCommandInbox,
};
use crate::viewport::animation::{PendingAnimationClip, UsdStageTime};
use crate::viewport::camera::{ArcballCamera, CameraMount, FlyTo};
use crate::viewport::scene::SelectedPrim;
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageHandle, StageInfo};

/// Installs the in-process implementation of the public viewport contract.
pub(crate) struct ViewportBridgePlugin;

/// Explicit ordering points around the public viewport bridge.
///
/// Native transports enqueue work before [`Self::ApplyCommands`] and drain
/// resulting events after [`Self::PublishStageLoadState`]. This preserves the
/// existing Frost behavior while avoiding a blocking pipe operation inside an
/// ECS command system.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ViewportBridgeSet {
    RefreshSceneIndex,
    ApplyCommands,
    ApplyTreeCommands,
    PublishStageLoadState,
}

impl Plugin for ViewportBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SceneQueryService>()
            .add_systems(Startup, emit_viewport_ready)
            .configure_sets(
                Update,
                (
                    ViewportBridgeSet::RefreshSceneIndex,
                    ViewportBridgeSet::ApplyCommands,
                    ViewportBridgeSet::ApplyTreeCommands,
                    ViewportBridgeSet::PublishStageLoadState,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                super::scene_index::refresh_scene_anchor_index
                    .in_set(ViewportBridgeSet::RefreshSceneIndex),
            )
            .add_systems(
                Update,
                (
                    publish_scene_query_results,
                    dispatch_scene_query_commands,
                    apply_viewport_commands,
                )
                    .chain()
                    .in_set(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                apply_tree_commands.in_set(ViewportBridgeSet::ApplyTreeCommands),
            )
            .add_systems(
                Update,
                publish_stage_load_state.in_set(ViewportBridgeSet::PublishStageLoadState),
            );
    }
}

fn publish_scene_query_results(
    query_service: Res<SceneQueryService>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    for result in query_service.drain_results() {
        outbox.push(ViewportEventEnvelope::new(
            Some(result.request_id),
            ViewportEvent::SearchResults {
                query: result.query,
                offset: result.offset,
                total: result.total,
                matches: result.matches,
                has_more: result.has_more,
            },
        ));
    }
}

fn emit_viewport_ready(mut outbox: ResMut<ViewportEventOutbox>) {
    outbox.push(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
}

fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    query_service: Res<SceneQueryService>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    for envelope in inbox.take_scene_query_commands() {
        let request_id = envelope.request_id;
        if envelope.protocol_version != PROTOCOL_VERSION {
            reject(
                &mut outbox,
                request_id,
                format!(
                    "unsupported protocol version {}; expected {}",
                    envelope.protocol_version, PROTOCOL_VERSION
                ),
            );
            continue;
        }

        match envelope.command {
            ViewportCommand::RequestSceneChildren {
                parent,
                page,
                page_size,
            } => outbox.push(ViewportEventEnvelope::new(
                Some(request_id),
                ViewportEvent::SceneChildren {
                    page: scene_index.children_page(parent.as_ref(), page, page_size),
                },
            )),
            ViewportCommand::SearchScene {
                query,
                offset,
                limit,
            } => {
                if !query_service.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    scene_index.nodes_snapshot(),
                ) {
                    reject(
                        &mut outbox,
                        request_id,
                        "scene search worker is unavailable".to_owned(),
                    );
                }
            }
            _ => unreachable!("scene query inbox only contains query commands"),
        }
    }
}

/// Emits lifecycle changes independently of who initiated the load. That
/// makes manual reloads, file-watcher reloads, and future host commands all
/// observable through the same public event.
fn publish_stage_load_state(
    asset_server: Res<AssetServer>,
    stage: Option<Res<StageHandle>>,
    spawned: Res<Spawned>,
    stage_info: Res<StageInfo>,
    selected: Res<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    camera_mount: Res<CameraMount>,
    clock: Res<UsdStageTime>,
    toggles: Res<DisplayToggles>,
    tuning: Res<LoaderTuning>,
    physics: Res<usd_bevy::physics::PhysicsActive>,
    mut last: Local<Option<(StageLoadState, u64)>>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    let state = match stage {
        None => StageLoadState::Idle,
        Some(stage) => match asset_server.get_load_state(&stage.0) {
            Some(LoadState::Failed(error)) => StageLoadState::Failed {
                message: error.to_string(),
            },
            Some(LoadState::Loaded) if spawned.0 => StageLoadState::Ready,
            _ => StageLoadState::Loading,
        },
    };
    let state_changed = last
        .as_ref()
        .is_none_or(|(previous, _)| previous != &state);
    let scene_changed = last
        .as_ref()
        .is_none_or(|(_, revision)| *revision != scene_index.revision());
    if state_changed || (matches!(state, StageLoadState::Ready) && scene_changed) {
        if state_changed {
            outbox.push(ViewportEventEnvelope::new(
                None,
                ViewportEvent::StageLoadStateChanged {
                    state: state.clone(),
                },
            ));
        }
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: build_read_model(
                    &stage_info,
                    spawned.0 && matches!(state, StageLoadState::Ready),
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                ),
            },
        ));
        *last = Some((state, scene_index.revision()));
    }
}

/// Applies commands whose state does not require a tree traversal. Tree
/// commands are forwarded to the next system after the scene index refreshes.
#[allow(clippy::too_many_arguments)]
fn apply_viewport_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut reload: ResMut<ReloadRequest>,
    mut selected: ResMut<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    mut tree_commands: ResMut<ViewportTreeCommandInbox>,
    mut camera_mount: ResMut<CameraMount>,
    mut clock: ResMut<UsdStageTime>,
    mut toggles: ResMut<DisplayToggles>,
    mut tuning: ResMut<LoaderTuning>,
    mut pending_animation: ResMut<PendingAnimationClip>,
    mut physics: ResMut<usd_bevy::physics::PhysicsActive>,
    stage: Option<Res<StageHandle>>,
    stage_info: Res<StageInfo>,
    spawned: Res<Spawned>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    while let Some(envelope) = inbox.pop() {
        let request_id = envelope.request_id.clone();
        if envelope.protocol_version != PROTOCOL_VERSION {
            reject(
                &mut outbox,
                request_id,
                format!(
                    "unsupported protocol version {}; expected {}",
                    envelope.protocol_version, PROTOCOL_VERSION
                ),
            );
            continue;
        }

        match envelope.command {
            ViewportCommand::RequestSnapshot => emit_snapshot(
                &mut outbox,
                request_id,
                &stage_info,
                &spawned,
                &selected,
                &scene_index,
                &camera_mount,
                &clock,
                &toggles,
                &tuning,
                physics.0,
            ),
            ViewportCommand::RequestSceneChildren { .. } | ViewportCommand::SearchScene { .. } => {
                reject(
                    &mut outbox,
                    request_id,
                    "scene query command was not dispatched".to_owned(),
                );
            }
            ViewportCommand::ReloadSession => {
                reload.requested = true;
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
            }
            ViewportCommand::SelectTarget { target } => {
                let selection = match target {
                    None => {
                        selected.0 = None;
                        SelectionReadModel { target: None }
                    }
                    Some(anchor) => match resolve_anchor(&anchor, &scene_index) {
                        Ok(entity) => {
                            selected.0 = Some(entity);
                            SelectionReadModel {
                                target: Some(anchor),
                            }
                        }
                        Err(reason) => {
                            reject(&mut outbox, request_id, reason);
                            continue;
                        }
                    },
                };
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::SelectionChanged { selection },
                ));
            }
            ViewportCommand::FocusTarget { target, mode } => {
                tree_commands.push(ViewportTreeCommand::Focus {
                    request_id,
                    target,
                    mode,
                });
            }
            ViewportCommand::SetSubtreeVisibility { target, visible } => {
                tree_commands.push(ViewportTreeCommand::SetSubtreeVisibility {
                    request_id,
                    target,
                    visible,
                });
            }
            ViewportCommand::SetVariantSelection {
                prim_path,
                set_name,
                option,
            } => {
                tuning
                    .variants
                    .insert((prim_path, set_name.clone()), option.clone());
                if set_name == "anim" {
                    pending_animation.name = Some(option);
                } else {
                    reload.requested = true;
                }
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
            }
            ViewportCommand::ResetVariantSelection {
                prim_path,
                set_name,
            } => {
                tuning
                    .variants
                    .remove(&(prim_path.clone(), set_name.clone()));
                if set_name == "anim" {
                    pending_animation.name = stage
                        .as_deref()
                        .and_then(|stage| usd_assets.get(&stage.0))
                        .into_iter()
                        .flat_map(|asset| asset.variants.iter())
                        .find(|(path, _)| *path == &prim_path)
                        .and_then(|(_, sets)| sets.iter().find(|set| set.name == set_name))
                        .and_then(|set| set.selection.clone());
                } else {
                    reload.requested = true;
                }
                emit_snapshot(
                    &mut outbox,
                    request_id,
                    &stage_info,
                    &spawned,
                    &selected,
                    &scene_index,
                    &camera_mount,
                    &clock,
                    &toggles,
                    &tuning,
                    physics.0,
                );
            }
            ViewportCommand::SetCameraSource { source } => {
                *camera_mount = match &source {
                    CameraSource::Arcball => CameraMount::Arcball,
                    CameraSource::Authored { prim_path } => CameraMount::Mounted {
                        prim_path: prim_path.clone(),
                    },
                };
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::CameraSourceChanged { source },
                ));
            }
            ViewportCommand::SetPlayback { playing } => {
                clock.playing = playing;
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::TimelineChanged {
                        timeline: timeline_read_model(&clock),
                    },
                ));
            }
            ViewportCommand::Seek { seconds } => {
                clock.seconds = seconds.clamp(0.0, clock.duration_seconds());
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::TimelineChanged {
                        timeline: timeline_read_model(&clock),
                    },
                ));
            }
            ViewportCommand::SetOverlay { overlay, enabled } => {
                set_overlay(&mut toggles, overlay, enabled);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetPrimMarkerBias { bias } => {
                toggles.prim_marker_bias = bias.clamp(0.0, 5.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetLightIntensity { scale } => {
                toggles.light_intensity_scale = scale.clamp(0.0, 5.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetCurveTuning { tuning: next } => {
                tuning.curves.default_radius = next.default_radius.clamp(0.001, 0.2);
                tuning.curves.ring_segments = next.ring_segments.clamp(3, 24);
                tuning.curves.point_scale = next.point_scale.clamp(0.05, 4.0);
                emit_presentation_changed(&mut outbox, request_id, &toggles, &tuning);
            }
            ViewportCommand::SetPhysicsRunning { running } => {
                physics.0 = running;
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::PhysicsChanged { running },
                ));
            }
        }
    }
}

/// Applies focus and visibility actions after scene anchors have been mapped
/// to their private Bevy entities. These implementations deliberately mirror
/// Frost's current tree behavior: a normal activation frames subtree bounds,
/// while a context-menu fly-to targets the prim origin at one quarter of the
/// current camera distance.
#[allow(clippy::too_many_arguments)]
fn apply_tree_commands(
    mut inbox: ResMut<ViewportTreeCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut selected: ResMut<SelectedPrim>,
    scene_index: Res<SceneAnchorIndex>,
    cameras: Query<&ArcballCamera>,
    transforms: Query<&GlobalTransform>,
    extents: Query<&usd_bevy::UsdLocalExtent>,
    children: Query<&Children>,
    mut visibility: Query<(Entity, &mut Visibility)>,
    mut fly_to: ResMut<FlyTo>,
) {
    while let Some(command) = inbox.pop() {
        match command {
            ViewportTreeCommand::Focus {
                request_id,
                target,
                mode,
            } => {
                let Some(entity) = scene_index.resolve(&target) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "target {} is not present in the active scene",
                            target.prim_path
                        ),
                    );
                    continue;
                };
                let Ok(camera) = cameras.single() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "cannot focus target before the active camera is ready".to_string(),
                    );
                    continue;
                };

                let (target_focus, target_distance) = match mode {
                    FocusMode::FlyToTarget => match transforms.get(entity) {
                        Ok(transform) => (
                            transform.translation(),
                            (camera.distance * 0.25).clamp(0.2, 40.0),
                        ),
                        Err(_) => {
                            reject(
                                &mut outbox,
                                request_id,
                                "cannot focus target before its transform is ready".to_string(),
                            );
                            continue;
                        }
                    },
                    FocusMode::FrameTarget => fit_params_for_entity(
                        entity,
                        &transforms,
                        &extents,
                        &children,
                        camera.distance,
                    ),
                };

                selected.0 = Some(entity);
                fly_to.start_focus = camera.focus;
                fly_to.start_distance = camera.distance;
                fly_to.target_focus = target_focus;
                fly_to.target_distance = target_distance;
                fly_to.start_yaw = None;
                fly_to.target_yaw = None;
                fly_to.start_elevation = None;
                fly_to.target_elevation = None;
                fly_to.duration = 0.4;
                fly_to.remaining = 0.4;

                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id.clone()),
                    ViewportEvent::SelectionChanged {
                        selection: SelectionReadModel {
                            target: Some(target.clone()),
                        },
                    },
                ));
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::CameraTransitionStarted { target, mode },
                ));
            }
            ViewportTreeCommand::SetSubtreeVisibility {
                request_id,
                target,
                visible,
            } => {
                let Some(root) = scene_index.resolve(&target) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "target {} is not present in the active scene",
                            target.prim_path
                        ),
                    );
                    continue;
                };

                set_subtree_visibility(root, &children, &mut visibility, visible);
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::PrimVisibilityChanged { target, visible },
                ));
            }
        }
    }
}

/// Matches the Frost tree's one-way descendant visibility change. Ancestors
/// and siblings remain untouched, and enabled descendants use `Visible`
/// rather than `Inherited` so a prior hidden parent cannot keep them hidden.
fn set_subtree_visibility(
    root: Entity,
    children: &Query<&Children>,
    visibility: &mut Query<(Entity, &mut Visibility)>,
    visible: bool,
) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok((_, mut current)) = visibility.get_mut(entity) {
            *current = if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter());
        }
    }
}

/// Frost's current subtree-bound calculation, retained verbatim in behavior
/// behind the protocol so a product client frames the same target the same way.
fn fit_params_for_entity(
    root: Entity,
    transforms: &Query<&GlobalTransform>,
    extents: &Query<&usd_bevy::UsdLocalExtent>,
    children: &Query<&Children>,
    current_camera_distance: f32,
) -> (Vec3, f32) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        if let (Ok(transform), Ok(extent)) = (transforms.get(entity), extents.get(entity)) {
            let matrix = transform.to_matrix();
            for index in 0..8 {
                let corner = Vec3::new(
                    if index & 1 == 0 {
                        extent.min[0]
                    } else {
                        extent.max[0]
                    },
                    if index & 2 == 0 {
                        extent.min[1]
                    } else {
                        extent.max[1]
                    },
                    if index & 4 == 0 {
                        extent.min[2]
                    } else {
                        extent.max[2]
                    },
                );
                let world_corner = matrix.transform_point3(corner);
                min = min.min(world_corner);
                max = max.max(world_corner);
            }
            found = true;
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter());
        }
    }

    if found {
        let center = (min + max) * 0.5;
        let size = (max - min).abs();
        let maximum_dimension = size.x.max(size.y).max(size.z).max(0.05);
        (center, (maximum_dimension * 1.6).clamp(0.2, 200.0))
    } else if let Ok(transform) = transforms.get(root) {
        (
            transform.translation(),
            (current_camera_distance * 0.25).clamp(0.2, 40.0),
        )
    } else {
        (Vec3::ZERO, current_camera_distance)
    }
}

fn resolve_anchor(anchor: &SceneAnchor, scene_index: &SceneAnchorIndex) -> Result<Entity, String> {
    scene_index.resolve(anchor).ok_or_else(|| {
        format!(
            "target {} is not present in the active scene",
            anchor.prim_path
        )
    })
}

fn set_overlay(toggles: &mut DisplayToggles, overlay: OverlayKind, enabled: bool) {
    match overlay {
        OverlayKind::GroundGrid => toggles.show_world_grid = enabled,
        OverlayKind::WorldAxes => toggles.show_world_axes = enabled,
        OverlayKind::PrimMarkers => toggles.show_prim_markers = enabled,
        OverlayKind::Skeleton => toggles.show_skeleton = enabled,
        OverlayKind::Physics => toggles.show_physics = enabled,
        OverlayKind::Colliders => toggles.show_colliders = enabled,
        OverlayKind::Wireframe => toggles.wireframe = enabled,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_snapshot(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    stage_info: &StageInfo,
    spawned: &Spawned,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::Snapshot {
            state: build_read_model(
                stage_info,
                spawned.0,
                selected,
                scene_index,
                camera_mount,
                clock,
                toggles,
                tuning,
                physics_running,
            ),
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn build_read_model(
    stage_info: &StageInfo,
    stage_loaded: bool,
    selected: &SelectedPrim,
    scene_index: &SceneAnchorIndex,
    camera_mount: &CameraMount,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) -> ViewportReadModel {
    ViewportReadModel {
        protocol_version: PROTOCOL_VERSION,
        stage: StageReadModel {
            display_name: stage_info.path.clone(),
            loaded: stage_loaded,
        },
        scene: scene_index.roots_read_model(),
        selection: SelectionReadModel {
            target: selected.0.and_then(|entity| scene_index.anchor_for(entity)),
        },
        camera_source: match camera_mount {
            CameraMount::Arcball => CameraSource::Arcball,
            CameraMount::Mounted { prim_path } => CameraSource::Authored {
                prim_path: prim_path.clone(),
            },
        },
        timeline: timeline_read_model(clock),
        presentation: presentation_read_model(toggles, tuning),
        physics_running,
    }
}

fn timeline_read_model(clock: &UsdStageTime) -> TimelineReadModel {
    TimelineReadModel {
        seconds: clock.seconds,
        playing: clock.playing,
        start_time_code: clock.start_time_code,
        end_time_code: clock.end_time_code,
        time_codes_per_second: clock.time_codes_per_second,
    }
}

fn presentation_read_model(
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) -> PresentationReadModel {
    PresentationReadModel {
        ground_grid: toggles.show_world_grid,
        world_axes: toggles.show_world_axes,
        prim_markers: toggles.show_prim_markers,
        prim_marker_bias: toggles.prim_marker_bias,
        skeleton: toggles.show_skeleton,
        physics: toggles.show_physics,
        colliders: toggles.show_colliders,
        wireframe: toggles.wireframe,
        light_intensity_scale: toggles.light_intensity_scale,
        curve_tuning: ProtocolCurveTuning {
            default_radius: tuning.curves.default_radius,
            ring_segments: tuning.curves.ring_segments,
            point_scale: tuning.curves.point_scale,
        },
    }
}

fn emit_presentation_changed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::PresentationChanged {
            presentation: presentation_read_model(toggles, tuning),
        },
    ));
}

fn reject(outbox: &mut ViewportEventOutbox, request_id: String, reason: String) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::CommandRejected { request_id, reason },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<SceneQueryService>()
            .init_resource::<ReloadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<CameraMount>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PendingAnimationClip>()
            .init_resource::<usd_bevy::physics::PhysicsActive>()
            .init_resource::<Spawned>()
            .init_resource::<Assets<UsdAsset>>()
            .insert_resource(StageInfo {
                path: "fixtures/spinner.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands);
        app
    }

    #[test]
    fn commands_update_runtime_state_and_publish_correlated_events() {
        let mut app = command_test_app();
        let request_ids = {
            let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
            vec![
                inbox.send(ViewportCommand::SetOverlay {
                    overlay: OverlayKind::Wireframe,
                    enabled: true,
                }),
                inbox.send(ViewportCommand::SetPlayback { playing: true }),
                inbox.send(ViewportCommand::Seek { seconds: 999.0 }),
                inbox.send(ViewportCommand::SetPhysicsRunning { running: true }),
            ]
        };

        app.update();

        assert!(app.world().resource::<DisplayToggles>().wireframe);
        assert!(app.world().resource::<UsdStageTime>().playing);
        assert_eq!(app.world().resource::<UsdStageTime>().seconds, 1.0 / 24.0);
        assert!(app.world().resource::<usd_bevy::physics::PhysicsActive>().0);

        let events: Vec<_> =
            std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
                .collect();
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0].request_id.as_deref(),
            Some(request_ids[0].as_str())
        );
        assert!(matches!(
            events[0].event,
            ViewportEvent::PresentationChanged { .. }
        ));
        assert_eq!(
            events[1].request_id.as_deref(),
            Some(request_ids[1].as_str())
        );
        assert!(matches!(
            events[1].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[2].request_id.as_deref(),
            Some(request_ids[2].as_str())
        );
        assert!(matches!(
            events[2].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[3].request_id.as_deref(),
            Some(request_ids[3].as_str())
        );
        assert!(matches!(
            events[3].event,
            ViewportEvent::PhysicsChanged { running: true }
        ));
    }

    #[test]
    fn snapshot_contains_only_logical_viewport_state() {
        let mut app = command_test_app();
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestSnapshot);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("snapshot command must emit a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::Snapshot { state } = event.event else {
            panic!("expected a snapshot event");
        };
        assert_eq!(state.stage.display_name, "fixtures/spinner.usda");
        assert!(state.scene.prims.is_empty());
        assert_eq!(state.selection.target, None);
    }
}
