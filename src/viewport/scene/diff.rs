//! Non-destructive semantic diff overlays for the current viewport.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use usd_diff::{EntityDiff, StageDiff};
use usd_model::{ChangeFlags, EntitySnapshot, PresenceState};

use crate::viewport::semantic::SemanticDiffState;

const ADDED_COLOR: Color = Color::srgb(0.20, 1.0, 0.35);
const CHANGED_COLOR: Color = Color::srgb(1.0, 0.68, 0.12);
const HISTORICAL_COLOR: Color = Color::srgba(1.0, 0.24, 0.48, 0.72);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OverlayPlan {
    current_added: bool,
    current_changed: bool,
    historical_removed: bool,
    historical_moved: bool,
}

fn overlay_plan(presence: PresenceState, flags: ChangeFlags) -> OverlayPlan {
    match presence {
        PresenceState::Added => OverlayPlan {
            current_added: true,
            ..Default::default()
        },
        PresenceState::Removed => OverlayPlan {
            historical_removed: true,
            ..Default::default()
        },
        PresenceState::Existing => OverlayPlan {
            current_changed: !flags.is_empty(),
            historical_moved: flags.contains(ChangeFlags::PATH),
            ..Default::default()
        },
    }
}

/// Draws semantic differences as gizmos only; authored and projected materials
/// remain untouched. Current objects are found by their current prim path,
/// while removed and moved objects use the historical semantic transform.
pub(crate) fn draw_semantic_diff(
    diff: Res<SemanticDiffState>,
    prims: Query<(&UsdPrimRef, &GlobalTransform, Option<&Aabb>)>,
    mut gizmos: Gizmos,
) {
    let Some(stage_diff) = diff.stage_diff() else {
        return;
    };
    let root_matrix = prims
        .iter()
        .find(|(prim, _, _)| prim.path == "/")
        .map(|(_, global, _)| global.compute_transform())
        .map(|transform| {
            Mat4::from_scale_rotation_translation(
                transform.scale,
                transform.rotation,
                transform.translation,
            )
        })
        .unwrap_or(Mat4::IDENTITY);

    for entity in stage_diff.entities.values() {
        let plan = overlay_plan(entity.presence, entity.flags);
        if plan.current_added {
            draw_current_entity(&mut gizmos, entity, &prims, ADDED_COLOR);
        }
        if plan.current_changed {
            draw_current_entity(&mut gizmos, entity, &prims, CHANGED_COLOR);
        }
        if plan.historical_removed
            && let Some(old) = entity.old.as_ref()
        {
            draw_historical_entity(&mut gizmos, stage_diff, old, root_matrix, HISTORICAL_COLOR);
        }
        if plan.historical_moved
            && let Some(old) = entity.old.as_ref()
        {
            draw_historical_entity(&mut gizmos, stage_diff, old, root_matrix, HISTORICAL_COLOR);
        }
    }
}

fn draw_current_entity(
    gizmos: &mut Gizmos,
    entity: &EntityDiff,
    prims: &Query<(&UsdPrimRef, &GlobalTransform, Option<&Aabb>)>,
    color: Color,
) {
    let Some(new) = entity.new.as_ref() else {
        return;
    };
    let Some((_, global, aabb)) = prims.iter().find(|(prim, _, _)| prim.path == new.prim_path)
    else {
        return;
    };

    let transform = global.compute_transform();
    let matrix = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    );
    if let Some(aabb) = aabb {
        draw_wire_box(
            gizmos,
            matrix,
            Vec3::from(aabb.center),
            Vec3::from(aabb.half_extents),
            color,
        );
    } else {
        draw_cross(gizmos, transform.translation, 0.2, color);
    }
}

fn draw_historical_entity(
    gizmos: &mut Gizmos,
    stage_diff: &StageDiff,
    old: &EntitySnapshot,
    root_matrix: Mat4,
    color: Color,
) {
    let matrix = historical_world_matrix(stage_diff, old, root_matrix);
    if let Some(geometry) = old.geometry.as_ref() {
        let bounds = geometry.local_bounds;
        let min = Vec3::new(
            bounds.min[0] as f32,
            bounds.min[1] as f32,
            bounds.min[2] as f32,
        );
        let max = Vec3::new(
            bounds.max[0] as f32,
            bounds.max[1] as f32,
            bounds.max[2] as f32,
        );
        draw_wire_box(gizmos, matrix, (min + max) * 0.5, (max - min) * 0.5, color);
    } else {
        draw_cross(gizmos, matrix.transform_point3(Vec3::ZERO), 0.2, color);
    }
}

pub(super) fn historical_world_matrix(
    stage_diff: &StageDiff,
    old: &EntitySnapshot,
    root: Mat4,
) -> Mat4 {
    let mut chain = Vec::new();
    let mut path = old.prim_path.as_str();
    while path != "/" {
        if let Some(ancestor) = stage_diff.entities.values().find_map(|entity| {
            entity
                .old
                .as_ref()
                .filter(|candidate| candidate.prim_path == path)
        }) {
            chain.push(ancestor);
        }
        let Some(parent) = parent_path(path) else {
            break;
        };
        path = parent;
    }
    chain.reverse();

    chain.into_iter().fold(root, |world, entity| {
        world * historical_local_matrix(entity)
    })
}

fn parent_path(path: &str) -> Option<&str> {
    let slash = path.rfind('/')?;
    if slash == 0 {
        Some("/")
    } else {
        Some(&path[..slash])
    }
}

fn historical_local_matrix(entity: &EntitySnapshot) -> Mat4 {
    let transform = historical_transform(entity);
    Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
}

fn historical_transform(old: &EntitySnapshot) -> Transform {
    let translation = old
        .transform
        .translation_mm
        .map(|value| value as f32 / 1_000.0);
    let rotation = old
        .transform
        .rotation_quantized
        .map(|value| value as f32 / 100_000.0);
    let scale = old
        .transform
        .scale_quantized
        .map(|value| value as f32 / 10_000.0);
    Transform {
        translation: Vec3::from_array(translation),
        rotation: Quat::from_xyzw(rotation[1], rotation[2], rotation[3], rotation[0]),
        scale: Vec3::from_array(scale),
    }
}

fn draw_wire_box(
    gizmos: &mut Gizmos,
    matrix: Mat4,
    center: Vec3,
    half_extents: Vec3,
    color: Color,
) {
    let corners = [
        Vec3::new(-half_extents.x, -half_extents.y, -half_extents.z),
        Vec3::new(half_extents.x, -half_extents.y, -half_extents.z),
        Vec3::new(half_extents.x, half_extents.y, -half_extents.z),
        Vec3::new(-half_extents.x, half_extents.y, -half_extents.z),
        Vec3::new(-half_extents.x, -half_extents.y, half_extents.z),
        Vec3::new(half_extents.x, -half_extents.y, half_extents.z),
        Vec3::new(half_extents.x, half_extents.y, half_extents.z),
        Vec3::new(-half_extents.x, half_extents.y, half_extents.z),
    ];
    let worldify = |corner: Vec3| matrix.transform_point3(center + corner);
    let corners: [Vec3; 8] = std::array::from_fn(|index| worldify(corners[index]));
    for (a, b) in [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        gizmos.line(corners[a], corners[b], color);
    }
}

fn draw_cross(gizmos: &mut Gizmos, origin: Vec3, half_length: f32, color: Color) {
    gizmos.line(
        origin - Vec3::X * half_length,
        origin + Vec3::X * half_length,
        color,
    );
    gizmos.line(
        origin - Vec3::Y * half_length,
        origin + Vec3::Y * half_length,
        color,
    );
    gizmos.line(
        origin - Vec3::Z * half_length,
        origin + Vec3::Z * half_length,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_plan_maps_presence_and_path_changes() {
        assert_eq!(
            overlay_plan(PresenceState::Added, ChangeFlags::empty()),
            OverlayPlan {
                current_added: true,
                ..Default::default()
            }
        );
        assert_eq!(
            overlay_plan(PresenceState::Removed, ChangeFlags::empty()),
            OverlayPlan {
                historical_removed: true,
                ..Default::default()
            }
        );
        assert_eq!(
            overlay_plan(PresenceState::Existing, ChangeFlags::PATH),
            OverlayPlan {
                current_changed: true,
                historical_moved: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn unchanged_entities_have_no_overlay_plan() {
        assert_eq!(
            overlay_plan(PresenceState::Existing, ChangeFlags::empty()),
            OverlayPlan::default()
        );
    }
}
