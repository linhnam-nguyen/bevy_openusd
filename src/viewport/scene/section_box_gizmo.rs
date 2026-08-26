//! Glacial interaction for the single aggregate Section Box transform.

use bevy::prelude::*;
use bevy_glacial::{
    gizmo::GizmoResult,
    prelude::{BoundsGizmoTarget, GizmoTarget},
};

use super::section_box::{
    SectionBoxClipPlanes, SectionBoxFace, SectionBoxPoseAuthority, SectionBoxState,
    resize_section_box_face,
};

#[derive(Component, Debug, Copy, Clone)]
pub(in crate::viewport) struct SectionBoxGizmoTarget;

/// Copies a transform changed by Glacial's Last-schedule gizmo pass into the
/// renderer-owned Section Box state. The target is the aggregate box, never a
/// selected USD entity, so interaction remains non-authoring and renderer-local.
pub(in crate::viewport) fn capture_section_box_gizmo_transform(
    mut state: ResMut<SectionBoxState>,
    targets: Query<(&Transform, &GizmoTarget), With<SectionBoxGizmoTarget>>,
) {
    let Ok((transform, target)) = targets.single() else {
        return;
    };
    if target.latest_result().is_none() && !target.is_active() {
        state.face_drag = None;
        return;
    }

    if target.is_active()
        && let Some(GizmoResult::ResizeFace { face, delta }) = target.latest_result()
    {
        apply_section_box_face_drag(
            &mut state,
            SectionBoxFace::from_bounds_face(face),
            delta as f32,
        );
        return;
    }

    state.face_drag = None;
    apply_section_box_gizmo_transform(&mut state, *transform);
}

fn apply_section_box_gizmo_transform(state: &mut SectionBoxState, transform: Transform) {
    if state.transform == transform {
        return;
    }

    state.transform = transform;
    state.clip_planes = SectionBoxClipPlanes::from_transform(transform);
    state.pose_authority = SectionBoxPoseAuthority::UserAdjusted;
    state.revision = state.revision.saturating_add(1);
}

fn apply_section_box_face_drag(state: &mut SectionBoxState, face: SectionBoxFace, delta: f32) {
    let start_transform = state
        .face_drag
        .filter(|drag| drag.face == face)
        .map_or(state.transform, |drag| drag.start_transform);
    let next_transform = resize_section_box_face(start_transform, face, delta);
    state.face_drag = Some(super::section_box::SectionBoxFaceDrag {
        face,
        start_transform,
    });
    if state.transform != next_transform {
        state.transform = next_transform;
        state.clip_planes = SectionBoxClipPlanes::from_transform(next_transform);
        state.pose_authority = SectionBoxPoseAuthority::UserAdjusted;
        state.revision = state.revision.saturating_add(1);
    }
}

/// Ensures that the effective Section Box has exactly one transient Glacial
/// target while visible and none while disabled or unresolved.
pub(in crate::viewport) fn sync_section_box_gizmo_target(
    mut commands: Commands,
    state: Res<SectionBoxState>,
    mut targets: Query<(Entity, &mut Transform), With<SectionBoxGizmoTarget>>,
) {
    if !state.enabled || !state.visible {
        for (entity, _) in &mut targets {
            commands.entity(entity).despawn();
        }
        return;
    }

    let mut retained = false;
    for (entity, mut transform) in &mut targets {
        if !retained {
            retained = true;
            if *transform != state.transform {
                *transform = state.transform;
            }
        } else {
            commands.entity(entity).despawn();
        }
    }

    if !retained {
        commands.spawn((
            Name::new("AggregateSectionBoxGizmo"),
            state.transform,
            GizmoTarget::default(),
            BoundsGizmoTarget,
            SectionBoxGizmoTarget,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewport_protocol::SceneAnchor;

    #[test]
    fn face_drag_promotes_autofit_to_user_adjusted_and_preserves_drag_baseline() {
        let mut state = SectionBoxState::default();
        state.enabled = true;
        state.visible = true;
        state.targets = vec![SceneAnchor::active_session("/World/Box")];
        state.transform = Transform::from_scale(Vec3::splat(10.0));
        let original_revision = state.revision;

        apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -3.0);
        let first = state.transform;
        assert_eq!(state.pose_authority, SectionBoxPoseAuthority::UserAdjusted);
        assert_eq!(
            state.face_drag.unwrap().start_transform.scale,
            Vec3::splat(10.0)
        );
        assert_eq!(state.transform.scale.x, 7.0);
        assert_eq!(state.transform.translation.x, -1.5);
        assert_eq!(state.revision, original_revision + 1);

        apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -4.0);
        assert_eq!(state.transform.scale.x, 6.0);
        assert_eq!(state.transform.translation.x, -2.0);
        assert_ne!(state.transform, first);

        state.reset_geometry();
        assert_eq!(state.pose_authority, SectionBoxPoseAuthority::AutoFit);
        assert!(state.face_drag.is_none());
    }

    #[test]
    fn face_adjusted_box_preserves_dimensions_through_translation_and_rotation() {
        let mut state = SectionBoxState::default();
        state.transform = Transform::from_scale(Vec3::splat(10.0));

        apply_section_box_face_drag(&mut state, SectionBoxFace::PositiveX, -3.0);
        let resized_scale = state.transform.scale;

        let translated = Transform {
            translation: Vec3::new(4.0, 5.0, 6.0),
            ..state.transform
        };
        apply_section_box_gizmo_transform(&mut state, translated);
        assert_eq!(state.transform.translation, translated.translation);
        assert_eq!(state.transform.scale, resized_scale);

        let rotated = Transform {
            rotation: Quat::from_rotation_y(0.5),
            ..state.transform
        };
        apply_section_box_gizmo_transform(&mut state, rotated);
        assert_eq!(state.transform.rotation, rotated.rotation);
        assert_eq!(state.transform.scale, resized_scale);
    }

    #[test]
    fn visible_state_spawns_exactly_one_aggregate_gizmo_target() {
        let mut app = App::new();
        app.init_resource::<SectionBoxState>()
            .add_systems(Update, sync_section_box_gizmo_target);
        let mut state = app.world_mut().resource_mut::<SectionBoxState>();
        state.enabled = true;
        state.visible = true;
        state.targets = vec![SceneAnchor::active_session("/World/Box")];
        state.transform = Transform::from_scale(Vec3::splat(2.0));
        state.bounds = None;
        state.clip_planes = SectionBoxClipPlanes::default();

        app.update();
        let count = app
            .world_mut()
            .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1);

        app.update();
        let count = app
            .world_mut()
            .query_filtered::<Entity, With<SectionBoxGizmoTarget>>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1);
    }
}
