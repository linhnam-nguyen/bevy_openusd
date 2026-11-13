//! Renderer-only shadow presentation for projected USD lights.
//!
//! The authored shadow capability is captured once per light. The viewer
//! toggle then gates Bevy's light components without writing back to USD.

use bevy::prelude::*;

use super::super::projection_work::{DirectProjectionWork, ProjectionBatch, ProjectionWork};
use super::{DisplayToggles, OriginalShadowEnabled};

#[derive(Resource, Debug, Clone, Copy)]
pub(super) struct ShadowProjectionState {
    applied_shadows: bool,
}

impl Default for ShadowProjectionState {
    fn default() -> Self {
        Self {
            applied_shadows: true,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/visualization_shadows_test_support.rs"]
mod test_support;
#[cfg(test)]
pub(super) use test_support::ShadowProjectionStats;

pub(super) fn capture_original_shadow_settings(
    toggles: Res<DisplayToggles>,
    mut cmds: Commands,
    mut dir: Query<
        (Entity, &mut DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalShadowEnabled>),
    >,
    mut pt: Query<(Entity, &mut PointLight), (Added<PointLight>, Without<OriginalShadowEnabled>)>,
    mut sp: Query<(Entity, &mut SpotLight), (Added<SpotLight>, Without<OriginalShadowEnabled>)>,
) {
    for (entity, mut light) in &mut dir {
        let authored = light.shadow_maps_enabled;
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored,
        );
        cmds.entity(entity).insert(OriginalShadowEnabled(authored));
    }
    for (entity, mut light) in &mut pt {
        let authored = light.shadow_maps_enabled;
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored,
        );
        cmds.entity(entity).insert(OriginalShadowEnabled(authored));
    }
    for (entity, mut light) in &mut sp {
        let authored = light.shadow_maps_enabled;
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored,
        );
        cmds.entity(entity).insert(OriginalShadowEnabled(authored));
    }
}

pub(super) fn apply_shadow_toggle(
    toggles: Res<DisplayToggles>,
    projection: ResMut<ShadowProjectionState>,
    lights: ParamSet<(
        Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
        Query<(&mut PointLight, &OriginalShadowEnabled)>,
        Query<(&mut SpotLight, &OriginalShadowEnabled)>,
    )>,
) {
    apply_shadow_toggle_core(toggles, projection, lights, &mut DirectProjectionWork);
}

#[allow(clippy::too_many_arguments)]
fn apply_shadow_toggle_core(
    toggles: Res<DisplayToggles>,
    mut projection: ResMut<ShadowProjectionState>,
    mut lights: ParamSet<(
        Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
        Query<(&mut PointLight, &OriginalShadowEnabled)>,
        Query<(&mut SpotLight, &OriginalShadowEnabled)>,
    )>,
    work: &mut impl ProjectionWork,
) {
    let desired = toggles.renderer.shadows;
    let toggle_changed = projection.applied_shadows != desired;
    projection.applied_shadows = desired;

    if toggle_changed {
        work.execute(
            ProjectionBatch::ShadowTransition,
            &mut lights.p0(),
            |(mut light, authored)| {
                set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
            },
        );
        work.execute(
            ProjectionBatch::ShadowTransition,
            &mut lights.p1(),
            |(mut light, authored)| {
                set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
            },
        );
        work.execute(
            ProjectionBatch::ShadowTransition,
            &mut lights.p2(),
            |(mut light, authored)| {
                set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
            },
        );
    }
}

fn set_shadow_enabled(current: &mut bool, desired: bool) {
    if *current != desired {
        *current = desired;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/visualization_shadows_tests.rs"]
mod tests;
