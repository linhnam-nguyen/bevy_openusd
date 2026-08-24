//! Glacial interaction for the single aggregate Section Box transform.

use bevy::prelude::*;
use bevy_glacial::prelude::GizmoTarget;

use super::section_box::{SectionBoxClipPlanes, SectionBoxState};

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
        return;
    }
    if state.transform == *transform {
        return;
    }

    state.transform = *transform;
    state.clip_planes = SectionBoxClipPlanes::from_transform(*transform);
    state.revision = state.revision.saturating_add(1);
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
            SectionBoxGizmoTarget,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewport_protocol::SceneAnchor;

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
