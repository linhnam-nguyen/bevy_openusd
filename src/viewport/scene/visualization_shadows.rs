//! Renderer-only shadow presentation for projected USD lights.
//!
//! The authored shadow capability is captured once per light. The viewer
//! toggle then gates Bevy's light components without writing back to USD.

use bevy::prelude::*;

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
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShadowProjectionStats {
    full_light_visits: u32,
}

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
    mut projection: ResMut<ShadowProjectionState>,
    mut lights: ParamSet<(
        Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
        Query<(&mut PointLight, &OriginalShadowEnabled)>,
        Query<(&mut SpotLight, &OriginalShadowEnabled)>,
    )>,
    #[cfg(test)] mut stats: Option<ResMut<ShadowProjectionStats>>,
) {
    let desired = toggles.renderer.shadows;
    let toggle_changed = projection.applied_shadows != desired;
    projection.applied_shadows = desired;

    if toggle_changed {
        for (mut light, authored) in &mut lights.p0() {
            #[cfg(test)]
            if let Some(stats) = stats.as_mut() {
                stats.full_light_visits += 1;
            }
            set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
        }
        for (mut light, authored) in &mut lights.p1() {
            #[cfg(test)]
            if let Some(stats) = stats.as_mut() {
                stats.full_light_visits += 1;
            }
            set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
        }
        for (mut light, authored) in &mut lights.p2() {
            #[cfg(test)]
            if let Some(stats) = stats.as_mut() {
                stats.full_light_visits += 1;
            }
            set_shadow_enabled(&mut light.shadow_maps_enabled, desired && authored.0);
        }
    }
}

fn set_shadow_enabled(current: &mut bool, desired: bool) {
    if *current != desired {
        *current = desired;
    }
}

#[cfg(test)]
#[path = "visualization_shadows_tests.rs"]
mod tests;
