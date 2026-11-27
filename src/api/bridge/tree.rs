use std::collections::HashSet;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use viewport_protocol::{CameraSource, SelectionReadModel, ViewportEvent, ViewportEventEnvelope};

use super::commands::VIEWER_BINDING_PENDING;
use super::focus_bounds::focus_bounds_for_payload;
use super::helpers::{emit_selection_delta, reject};
use crate::api::{ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox};
use crate::viewport::navigation::camera::{
    ArcballCamera, CameraMount, CameraNavigationConfig, FlyTo, FramingIntent, FramingOutcome,
    frame_intent_for_camera,
};
use crate::viewport::rendering::scene::{
    CurrentHierarchyProjection, HierarchyVisibilityTarget, SceneAnchorIndex,
};
use crate::viewport::rendering::scene::{SelectedPrim, SelectedTargets};

/// Applies focus and visibility actions after scene anchors have been mapped
/// to their private Bevy entities. Focus is latest-value work tagged with the
/// selection and scene revisions captured when it was requested; visibility
/// commands stay ahead of a deferred focus retry.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_tree_commands(
    mut inbox: ResMut<ViewportTreeCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut selected: ResMut<SelectedPrim>,
    mut selection: ResMut<SelectedTargets>,
    (mut scene_index, live_stage, nav_config, mut camera_mount): (
        ResMut<SceneAnchorIndex>,
        Option<NonSend<usd_bevy::LiveStage>>,
        Option<Res<CameraNavigationConfig>>,
        Option<ResMut<CameraMount>>,
    ),
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    cameras: Query<
        (&ArcballCamera, Option<&Projection>, Option<&Camera>),
        Without<usd_bevy::UsdCamera>,
    >,
    transforms: Query<&Transform>,
    child_of: Query<Option<&ChildOf>>,
    extents: Query<&usd_bevy::UsdLocalExtent>,
    aabbs: Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: Query<Option<&Mesh3d>>,
    children: Query<&Children>,
    mut visibility: Query<(Entity, &mut Visibility)>,
    mut fly_to: ResMut<FlyTo>,
    gate: crate::viewport::navigation::input::ViewerInteractionGate,
) {
    let locked = gate.locked();
    let config = nav_config.as_deref().copied().unwrap_or_default();
    while let Some(command) = inbox.pop() {
        if locked && matches!(command, ViewportTreeCommand::Focus { .. }) {
            let ViewportTreeCommand::Focus { request_id, .. } = command else {
                unreachable!("focus guard matched non-focus command");
            };
            reject(&mut outbox, request_id, VIEWER_BINDING_PENDING.to_owned());
            continue;
        }
        match command {
            ViewportTreeCommand::Focus {
                request_id,
                target,
                mode,
                selection_revision,
                scene_revision,
                generation,
            } => {
                if selection_revision != selection.revision()
                    || scene_revision != scene_index.revision()
                {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "focus request generation {generation} was superseded by newer scene state"
                        ),
                    );
                    continue;
                }
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
                let Ok((camera, projection, bevy_camera)) = cameras.single() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "cannot focus target before the active camera is ready".to_string(),
                    );
                    continue;
                };

                let canonical = live_stage.as_deref().map_or(Ok(None), |stage| {
                    let unique_prim = target.instance_context.is_none()
                        && matches!(
                            scene_index.resolve_all_by_prim_path(&target.prim_path),
                            [only] if *only == entity
                        );
                    focus_bounds_for_payload(stage, &target.prim_path, unique_prim)
                });
                let canonical = match canonical {
                    Ok(bounds) => bounds,
                    Err(reason) => {
                        reject(&mut outbox, request_id, reason.to_owned());
                        continue;
                    }
                };
                let focus_bounds = if let Some((min, max)) = canonical {
                    EntityFocusBounds::Bounds(min, max)
                } else {
                    entity_bounds_for_focus(
                        entity,
                        &transforms,
                        &child_of,
                        &extents,
                        &aabbs,
                        &meshes,
                        &children,
                    )
                };
                let Some(active_projection) = projection else {
                    reject(
                        &mut outbox,
                        request_id,
                        "cannot focus target before the active camera projection is ready"
                            .to_string(),
                    );
                    continue;
                };
                let viewport_size = bevy_camera.and_then(|c| {
                    c.logical_viewport_size()
                        .or_else(|| c.physical_viewport_size().map(|size| size.as_vec2()))
                });
                let intent = match focus_bounds {
                    EntityFocusBounds::Bounds(min, max) => FramingIntent::Bounds { min, max },
                    EntityFocusBounds::Point(origin) => FramingIntent::Point(origin),
                    EntityFocusBounds::Pending | EntityFocusBounds::Missing => {
                        // A Mesh3d can exist for a frame before Bevy has produced
                        // its Aabb. Preserve the command and retry next frame so
                        // the camera never commits to the prim origin as a fake
                        // fit target.
                        inbox.defer_focus(ViewportTreeCommand::Focus {
                            request_id,
                            target,
                            mode,
                            selection_revision,
                            scene_revision,
                            generation,
                        });
                        break;
                    }
                };
                let fit = match frame_intent_for_camera(
                    intent,
                    camera,
                    active_projection,
                    viewport_size,
                    &config,
                ) {
                    FramingOutcome::Fit(fit) => fit,
                    FramingOutcome::Deferred => {
                        inbox.defer_focus(ViewportTreeCommand::Focus {
                            request_id,
                            target,
                            mode,
                            selection_revision,
                            scene_revision,
                            generation,
                        });
                        break;
                    }
                    FramingOutcome::Unrepresentable => {
                        reject(
                            &mut outbox,
                            request_id,
                            format!(
                                "target {} bounds cannot be framed within the active view envelope",
                                target.prim_path
                            ),
                        );
                        continue;
                    }
                };

                // Transactional commit: only after framing succeeds do we
                // switch from a mounted authored camera to free mode and
                // install the complete transition state.
                let was_mounted = camera_mount
                    .as_deref()
                    .is_some_and(|mount| matches!(mount, CameraMount::Mounted { .. }));
                if let Some(mount) = camera_mount.as_deref_mut() {
                    *mount = CameraMount::Arcball;
                }
                selected.0 = Some(entity);
                selection
                    .replace(SelectionReadModel::from_legacy_target(Some(target.clone())))
                    .expect("focused target must satisfy the protocol invariant");
                *fly_to = FlyTo {
                    start_focus: camera.focus,
                    start_distance: camera.distance,
                    target_focus: fit.focus,
                    target_distance: fit.distance,
                    start_yaw: None,
                    target_yaw: None,
                    start_elevation: None,
                    target_elevation: None,
                    start_ortho_scale: match active_projection {
                        Projection::Orthographic(ortho) => Some(ortho.scale),
                        _ => None,
                    },
                    target_ortho_scale: fit.orthographic.map(|ortho| ortho.scale),
                    framing_intent: Some(intent),
                    duration: 0.4,
                    remaining: 0.4,
                };

                if was_mounted {
                    outbox.push(ViewportEventEnvelope::new(
                        Some(request_id.clone()),
                        ViewportEvent::CameraSourceChanged {
                            source: CameraSource::Arcball,
                        },
                    ));
                }
                emit_selection_delta(request_id.clone(), &selection, &mut outbox);
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
                if !scene_index.contains_logical_anchor(&target) {
                    reject(
                        &mut outbox,
                        request_id,
                        format!(
                            "target {} is not present in the active scene",
                            target.prim_path
                        ),
                    );
                    continue;
                }

                // Record logical visibility intent regardless of whether the
                // target's ECS entity is currently resident, and apply to
                // resident entities immediately when present.
                let _ = scene_index.set_logical_subtree_visibility(&target, visible);
                if let Some(root) = scene_index.resolve(&target) {
                    set_subtree_visibility(root, &children, &mut visibility, visible);
                }
                for (entity, effective) in scene_index.resident_visibility_intents() {
                    if let Ok((_, mut state)) = visibility.get_mut(entity) {
                        *state = if effective { Visibility::Visible } else { Visibility::Hidden };
                    }
                }
                if current_projection.source() == viewport_protocol::HierarchySource::Prim {
                    let node_id = viewport_protocol::HierarchyNodeId::for_scene_anchor(&target);
                    let _ = current_projection.apply_visibility(&node_id, visible);
                }
                outbox.push(ViewportEventEnvelope::new(
                    Some(request_id),
                    ViewportEvent::PrimVisibilityChanged { target, visible },
                ));
            }
            ViewportTreeCommand::SetHierarchyNodeVisibility {
                request_id,
                source,
                node_id,
                visible,
            } => {
                if source != current_projection.source() {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy provider {source:?} is not active"),
                    );
                    continue;
                }
                let Some(targets) = current_projection.visibility_targets(&node_id) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy node {} is not present", node_id.as_str()),
                    );
                    continue;
                };
                let targets = targets.to_vec();
                if scene_index.has_logical_authority()
                    && (!scene_index.logical_hierarchy_available()
                        || targets.iter().any(|target| match target {
                            HierarchyVisibilityTarget::SceneAnchor(anchor) => {
                                !scene_index.contains_logical_anchor(anchor)
                            }
                            HierarchyVisibilityTarget::PrimPath(path) => {
                                !scene_index.accepts_logical_prim_path(path)
                            }
                        }))
                {
                    reject(&mut outbox, request_id,
                        "Hierarchy target is not qualified in the active occurrence graph.".to_owned());
                    continue;
                }
                let mut roots = HashSet::new();
                let mut logical_paths = Vec::new();
                for target in targets {
                    match target {
                        HierarchyVisibilityTarget::SceneAnchor(anchor) => {
                            let _ = scene_index.set_logical_subtree_visibility(&anchor, visible);
                            if let Some(entity) = scene_index.resolve(&anchor) {
                                roots.insert(entity);
                            }
                        }
                        HierarchyVisibilityTarget::PrimPath(path) => {
                            roots.extend(
                                scene_index.resolve_all_by_prim_path(&path).iter().copied(),
                            );
                            logical_paths.push(path);
                        }
                    }
                }
                if !logical_paths.is_empty() {
                    scene_index.set_logical_prim_paths_visibility(&logical_paths, visible);
                }
                set_subtree_visibility_for_roots(roots, &children, &mut visibility, visible);
                for (entity, effective) in scene_index.resident_visibility_intents() {
                    if let Ok((_, mut state)) = visibility.get_mut(entity) {
                        *state = if effective { Visibility::Visible } else { Visibility::Hidden };
                    }
                }
                let Some(event) = current_projection.apply_visibility(&node_id, visible) else {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy node {} could not be updated", node_id.as_str()),
                    );
                    continue;
                };
                outbox.push(ViewportEventEnvelope::new(Some(request_id), event));
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
    set_subtree_visibility_for_roots([root], children, visibility, visible);
}

fn set_subtree_visibility_for_roots(
    roots: impl IntoIterator<Item = Entity>,
    children: &Query<&Children>,
    visibility: &mut Query<(Entity, &mut Visibility)>,
    visible: bool,
) {
    let mut stack: Vec<Entity> = roots.into_iter().collect();
    let mut visited = HashSet::new();
    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) {
            continue;
        }
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

enum EntityFocusBounds {
    Bounds(Vec3, Vec3),
    Point(Vec3),
    Pending,
    Missing,
}

/// Collects the world-space subtree bounds used by focus commands before
/// delegating framing to the shared viewport camera policy.
fn entity_bounds_for_focus(
    root: Entity,
    transforms: &Query<&Transform>,
    child_of: &Query<Option<&ChildOf>>,
    extents: &Query<&usd_bevy::UsdLocalExtent>,
    aabbs: &Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: &Query<Option<&Mesh3d>>,
    children: &Query<&Children>,
) -> EntityFocusBounds {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    let mut mesh_bounds_pending = false;
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        if transforms.get(entity).is_ok() {
            let Some(matrix) = world_matrix(entity, transforms, child_of) else {
                return EntityFocusBounds::Missing;
            };
            if let Ok(extent) = extents.get(entity) {
                include_bounds(
                    &mut min,
                    &mut max,
                    matrix,
                    Vec3::from_array(extent.min),
                    Vec3::from_array(extent.max),
                );
                found = true;
            } else if let Ok(Some(aabb)) = aabbs.get(entity) {
                include_bounds(
                    &mut min,
                    &mut max,
                    matrix,
                    Vec3::from(aabb.center - aabb.half_extents),
                    Vec3::from(aabb.center + aabb.half_extents),
                );
                found = true;
            } else if meshes.get(entity).ok().flatten().is_some() {
                mesh_bounds_pending = true;
            }
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter());
        }
    }

    if found && min.is_finite() && max.is_finite() && min.cmple(max).all() {
        EntityFocusBounds::Bounds(min, max)
    } else if mesh_bounds_pending {
        EntityFocusBounds::Pending
    } else if transforms.get(root).is_ok() {
        let Some(matrix) = world_matrix(root, transforms, child_of) else {
            return EntityFocusBounds::Missing;
        };
        let origin = matrix.transform_point3(Vec3::ZERO);
        if origin.is_finite() {
            EntityFocusBounds::Point(origin)
        } else {
            EntityFocusBounds::Missing
        }
    } else {
        EntityFocusBounds::Missing
    }
}

/// Computes the current world matrix from local transforms instead of relying
/// on `GlobalTransform`, which is propagated later in the frame. This keeps a
/// selection command correct even when it arrives in the same frame as an
/// authored transform update.
fn world_matrix(
    entity: Entity,
    transforms: &Query<&Transform>,
    child_of: &Query<Option<&ChildOf>>,
) -> Option<Mat4> {
    let mut chain = Vec::new();
    let mut current = Some(entity);
    let mut guard = 0usize;
    while let Some(entity) = current {
        let transform = transforms.get(entity).ok()?;
        chain.push(transform.to_matrix());
        current = child_of.get(entity).ok().flatten().map(ChildOf::parent);
        guard += 1;
        if guard > 10_000 {
            return None;
        }
    }

    Some(
        chain
            .into_iter()
            .rev()
            .fold(Mat4::IDENTITY, |parent, local| parent * local),
    )
}

fn include_bounds(min: &mut Vec3, max: &mut Vec3, matrix: Mat4, local_min: Vec3, local_max: Vec3) {
    for index in 0..8 {
        let corner = Vec3::new(
            if index & 1 == 0 {
                local_min.x
            } else {
                local_max.x
            },
            if index & 2 == 0 {
                local_min.y
            } else {
                local_max.y
            },
            if index & 4 == 0 {
                local_min.z
            } else {
                local_max.z
            },
        );
        let world_corner = matrix.transform_point3(corner);
        if world_corner.is_finite() {
            *min = min.min(world_corner);
            *max = max.max(world_corner);
        }
    }
}
