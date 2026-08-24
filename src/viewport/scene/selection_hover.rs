//! Renderer-local hover target tracking.
//!
//! Pointer motion remains an input detail. This module resolves only the
//! nearest changed target into a stable [`SceneAnchor`] for presentation.

use bevy::ecs::hierarchy::ChildOf;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::SceneAnchorIndex;
use crate::viewport::input::ViewportNavigationInput;

/// Current renderer-local hover target and the inputs used for its last pick.
///
/// The cache fields are deliberately renderer-local. Only `anchor` is read by
/// the presentation system, and raw pointer coordinates never cross the
/// viewport protocol a second time.
#[derive(Resource, Debug, Default, Clone)]
pub(super) struct HoveredTarget {
    pub(super) anchor: Option<SceneAnchor>,
    last_pointer_position: Option<Vec2>,
    last_viewport_size: Option<Vec2>,
    last_camera_local: Option<Transform>,
    last_camera_global: Option<Transform>,
    last_clip_from_view: Option<Mat4>,
    last_camera_viewport_size: Option<Vec2>,
    last_focused: Option<bool>,
    last_scene_revision: Option<u64>,
}

/// Renderer diagnostics used by the B5 performance evidence packet.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HoverPickStats {
    pub(crate) raycasts: u64,
    pub(crate) skipped_idle_frames: u64,
}

/// Resolves the current cursor to the nearest projected mesh triangle.
///
/// Bevy's mesh ray caster uses entity AABBs only to cull candidates and then
/// intersects the actual mesh triangles. The filter excludes non-projected
/// meshes, while `anchor_for_hit` maps projected mesh children back to the
/// owning stable scene anchor.
pub(super) fn update_hover_target(
    input: Res<ViewportNavigationInput>,
    scene_index: Res<SceneAnchorIndex>,
    mut hovered: ResMut<HoveredTarget>,
    mut stats: ResMut<HoverPickStats>,
    cameras: Query<(&Camera, &GlobalTransform, &Transform), With<Camera3d>>,
    child_of: Query<&ChildOf>,
    mut mesh_ray_cast: MeshRayCast,
) {
    let Ok((camera, camera_global, camera_local)) = cameras.single() else {
        return;
    };

    let camera_local = *camera_local;
    let camera_global_transform = camera_global.compute_transform();
    let clip_from_view = camera.clip_from_view();
    let Some(camera_viewport_size) = camera.logical_viewport_size() else {
        return;
    };
    let pointer_changed = hovered.last_pointer_position != input.pointer_position;
    let viewport_changed = hovered.last_viewport_size != Some(input.viewport_size);
    let camera_changed = hovered.last_camera_local != Some(camera_local)
        || hovered.last_camera_global != Some(camera_global_transform)
        || hovered.last_clip_from_view != Some(clip_from_view)
        || hovered.last_camera_viewport_size != Some(camera_viewport_size);
    let focus_changed = hovered.last_focused != Some(input.focused);
    let scene_changed = hovered.last_scene_revision != Some(scene_index.revision());

    if !pointer_changed && !viewport_changed && !camera_changed && !focus_changed && !scene_changed
    {
        stats.skipped_idle_frames = stats.skipped_idle_frames.saturating_add(1);
        return;
    }

    hovered.last_pointer_position = input.pointer_position;
    hovered.last_viewport_size = Some(input.viewport_size);
    hovered.last_camera_local = Some(camera_local);
    hovered.last_camera_global = Some(camera_global_transform);
    hovered.last_clip_from_view = Some(clip_from_view);
    hovered.last_camera_viewport_size = Some(camera_viewport_size);
    hovered.last_focused = Some(input.focused);
    hovered.last_scene_revision = Some(scene_index.revision());
    stats.raycasts = stats.raycasts.saturating_add(1);

    let next = if !input.focused {
        None
    } else if let Some(pointer) = input.pointer_position {
        let pointer =
            map_pointer_to_camera_viewport(pointer, input.viewport_size, camera_viewport_size);
        camera
            .viewport_to_world(camera_global, pointer)
            .ok()
            .and_then(|ray| {
                let filter = |entity| anchor_for_hit(entity, &scene_index, &child_of).is_some();
                let settings = MeshRayCastSettings::default().with_filter(&filter);
                mesh_ray_cast
                    .cast_ray(ray, &settings)
                    .iter()
                    .find_map(|(entity, _hit)| anchor_for_hit(*entity, &scene_index, &child_of))
            })
    } else {
        None
    };

    if hovered.anchor != next {
        hovered.anchor = next;
    }
}

/// Maps the browser's CSS-pixel cursor into the actual logical Bevy camera
/// viewport. Remote streams may render at a device-pixel or capped size that
/// differs from the CSS video size, while native input normally has a 1:1
/// mapping.
fn map_pointer_to_camera_viewport(
    pointer: Vec2,
    input_viewport_size: Vec2,
    camera_viewport_size: Vec2,
) -> Vec2 {
    let input_viewport_size = input_viewport_size.max(Vec2::splat(f32::EPSILON));
    (pointer / input_viewport_size * camera_viewport_size).clamp(Vec2::ZERO, camera_viewport_size)
}

/// Resolves a mesh hit on a projected child entity to its owning prim.
fn anchor_for_hit(
    mut entity: Entity,
    scene_index: &SceneAnchorIndex,
    child_of: &Query<&ChildOf>,
) -> Option<SceneAnchor> {
    // The hierarchy is authored by scene projection and cannot contain a
    // cycle. The bound keeps malformed external ECS state from spinning the
    // hover system forever.
    for _ in 0..256 {
        if let Some(anchor) = scene_index.anchor_for(entity) {
            return Some(anchor);
        }
        entity = child_of.get(entity).ok()?.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_pick_state_requires_no_new_input_or_camera_revision() {
        let input = ViewportNavigationInput::default();
        let mut state = HoveredTarget {
            last_pointer_position: input.pointer_position,
            last_viewport_size: Some(input.viewport_size),
            last_camera_local: Some(Transform::IDENTITY),
            last_camera_global: Some(Transform::IDENTITY),
            last_clip_from_view: Some(Mat4::IDENTITY),
            last_focused: Some(input.focused),
            last_scene_revision: Some(4),
            ..default()
        };

        assert_eq!(state.last_pointer_position, input.pointer_position);
        assert_eq!(state.last_viewport_size, Some(input.viewport_size));
        assert_eq!(state.last_scene_revision, Some(4));

        state.last_pointer_position = Some(Vec2::new(4.0, 2.0));
        assert_ne!(state.last_pointer_position, input.pointer_position);
    }

    #[test]
    fn hover_pick_stats_start_without_raycasts() {
        assert_eq!(HoverPickStats::default().raycasts, 0);
        assert_eq!(HoverPickStats::default().skipped_idle_frames, 0);
    }

    #[test]
    fn remote_css_pointer_maps_to_actual_camera_viewport() {
        let mapped = map_pointer_to_camera_viewport(
            Vec2::new(480.0, 270.0),
            Vec2::new(960.0, 540.0),
            Vec2::new(1920.0, 1080.0),
        );
        assert!(mapped.abs_diff_eq(Vec2::new(960.0, 540.0), 1e-5));
    }

    #[test]
    fn remote_css_pointer_maps_non_two_x_camera_viewport() {
        let mapped = map_pointer_to_camera_viewport(
            Vec2::new(375.0, 225.0),
            Vec2::new(1000.0, 600.0),
            Vec2::new(1500.0, 900.0),
        );
        assert!(mapped.abs_diff_eq(Vec2::new(562.5, 337.5), 1e-5));
    }
}
