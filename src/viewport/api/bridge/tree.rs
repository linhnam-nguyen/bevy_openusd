use std::collections::HashSet;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use viewport_protocol::{SelectionReadModel, ViewportEvent, ViewportEventEnvelope};

use super::helpers::{emit_selection_delta, reject};
use crate::viewport::api::{
    CurrentHierarchyProjection, HierarchyVisibilityTarget, SceneAnchorIndex, ViewportEventOutbox,
    ViewportTreeCommand, ViewportTreeCommandInbox,
};
use crate::viewport::camera::{ArcballCamera, FlyTo};
use crate::viewport::scene::{SelectedPrim, SelectedTargets};

/// Applies focus and visibility actions after scene anchors have been mapped
/// to their private Bevy entities. Both selection and fly-to use the same
/// subtree bounds, so repeating the action does not progressively zoom toward
/// a prim's transform origin.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_tree_commands(
    mut inbox: ResMut<ViewportTreeCommandInbox>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut selected: ResMut<SelectedPrim>,
    mut selection: ResMut<SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    cameras: Query<&ArcballCamera>,
    transforms: Query<&Transform>,
    child_of: Query<Option<&ChildOf>>,
    extents: Query<&usd_bevy::UsdLocalExtent>,
    aabbs: Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: Query<Option<&Mesh3d>>,
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

                let Some((target_focus, target_distance)) = fit_params_for_entity(
                    entity,
                    &transforms,
                    &child_of,
                    &extents,
                    &aabbs,
                    &meshes,
                    &children,
                    camera.distance,
                ) else {
                    // A Mesh3d can exist for a frame before Bevy has produced
                    // its Aabb. Preserve the command and retry next frame so
                    // the camera never commits to the prim origin as a fake
                    // fit target.
                    inbox.push_front(ViewportTreeCommand::Focus {
                        request_id,
                        target,
                        mode,
                    });
                    break;
                };

                selected.0 = Some(entity);
                selection
                    .replace(SelectionReadModel::from_legacy_target(Some(target.clone())))
                    .expect("focused target must satisfy the protocol invariant");
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
                let mut roots = HashSet::new();
                for target in targets {
                    match target {
                        HierarchyVisibilityTarget::SceneAnchor(anchor) => {
                            if let Some(entity) = scene_index.resolve(&anchor) {
                                roots.insert(entity);
                            }
                        }
                        HierarchyVisibilityTarget::PrimPath(path) => {
                            roots.extend(
                                scene_index.resolve_all_by_prim_path(&path).iter().copied(),
                            );
                        }
                    }
                }
                set_subtree_visibility_for_roots(roots, &children, &mut visibility, visible);
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

/// Computes the subtree bounds used by both public focus modes so a product
/// client frames the same target the same way.
fn fit_params_for_entity(
    root: Entity,
    transforms: &Query<&Transform>,
    child_of: &Query<Option<&ChildOf>>,
    extents: &Query<&usd_bevy::UsdLocalExtent>,
    aabbs: &Query<Option<&bevy::camera::primitives::Aabb>>,
    meshes: &Query<Option<&Mesh3d>>,
    children: &Query<&Children>,
    current_camera_distance: f32,
) -> Option<(Vec3, f32)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    let mut mesh_bounds_pending = false;
    let mut stack = vec![root];

    while let Some(entity) = stack.pop() {
        if transforms.get(entity).is_ok() {
            let matrix = world_matrix(entity, transforms, child_of)?;
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

    if found {
        let center = (min + max) * 0.5;
        let size = (max - min).abs();
        let maximum_dimension = size.x.max(size.y).max(size.z).max(0.05);
        Some((center, (maximum_dimension * 1.6).clamp(0.2, 10_000.0)))
    } else if mesh_bounds_pending {
        None
    } else if transforms.get(root).is_ok() {
        Some((
            world_matrix(root, transforms, child_of)?.transform_point3(Vec3::ZERO),
            current_camera_distance.clamp(0.2, 10_000.0),
        ))
    } else {
        None
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
        *min = min.min(world_corner);
        *max = max.max(world_corner);
    }
}
