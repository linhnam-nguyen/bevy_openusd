//! Renderer-local hover target tracking.
//!
//! Pointer motion remains an input detail. This module resolves only the
//! nearest changed target into a stable [`SceneAnchor`] for presentation.

use bevy::camera::primitives::Aabb;
use bevy::math::bounding::{Aabb3d, RayCast3d};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::SceneAnchorIndex;
use crate::viewport::input::ViewportNavigationInput;

/// Current renderer-local hover target. It is never sent as raw pointer data
/// through the viewport protocol.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HoveredTarget {
    pub(super) anchor: Option<SceneAnchor>,
}

/// Resolves the current cursor to the nearest projected prim AABB.
///
/// The resource is mutated only when the resolved anchor changes, which keeps
/// hover presentation local and avoids broad reactive/server state churn.
pub(super) fn update_hover_target(
    input: Res<ViewportNavigationInput>,
    scene_index: Res<SceneAnchorIndex>,
    mut hovered: ResMut<HoveredTarget>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prims: Query<(
        Entity,
        &UsdPrimRef,
        &GlobalTransform,
        Option<&Aabb>,
        &Mesh3d,
    )>,
) {
    let next = resolve_hovered_anchor(&input, &scene_index, &cameras, &prims);
    if hovered.anchor != next {
        hovered.anchor = next;
    }
}

fn resolve_hovered_anchor(
    input: &ViewportNavigationInput,
    scene_index: &SceneAnchorIndex,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prims: &Query<(
        Entity,
        &UsdPrimRef,
        &GlobalTransform,
        Option<&Aabb>,
        &Mesh3d,
    )>,
) -> Option<SceneAnchor> {
    if !input.focused {
        return None;
    }
    let pointer = input.pointer_position?;
    let (camera, camera_transform) = cameras.single().ok()?;
    let ray = camera.viewport_to_world(camera_transform, pointer).ok()?;

    prims
        .iter()
        .filter_map(|(entity, _prim, global, aabb, _mesh)| {
            let aabb = aabb?;
            let world_aabb = world_aabb(global, aabb);
            let distance = RayCast3d::from_ray(ray, f32::MAX).aabb_intersection_at(&world_aabb)?;
            let anchor = scene_index.anchor_for(entity)?;
            Some((distance, anchor))
        })
        .min_by(
            |(left_distance, left_anchor), (right_distance, right_anchor)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| left_anchor.cmp(right_anchor))
            },
        )
        .map(|(_, anchor)| anchor)
}

fn world_aabb(global: &GlobalTransform, local: &Aabb) -> Aabb3d {
    let transform = global.compute_transform();
    let center = Vec3::from(local.center);
    let half_extents = Vec3::from(local.half_extents);
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);

    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let local_point = center + half_extents * Vec3::new(x, y, z);
                let world_point = transform.transform_point(local_point);
                minimum = minimum.min(world_point);
                maximum = maximum.max(world_point);
            }
        }
    }

    Aabb3d::from_min_max(minimum, maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformed_aabb_contains_rotated_corners() {
        let global = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(
            90.0_f32.to_radians(),
        )));
        let local = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::new(1.0, 2.0, 3.0),
        };
        let world = world_aabb(&global, &local);
        assert!(world.min.x <= -2.99 && world.max.x >= 2.99);
        assert!(world.min.y <= -1.99 && world.max.y >= 1.99);
        assert!(world.min.z <= -0.99 && world.max.z >= 0.99);
    }
}
