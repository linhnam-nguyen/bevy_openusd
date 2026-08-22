//! Renderer-only shadow presentation for projected USD lights.
//!
//! The authored shadow capability is captured once per light. The viewer
//! toggle then gates Bevy's light components without writing back to USD.

use bevy::prelude::*;

use super::{DisplayToggles, OriginalShadowEnabled};

pub(super) fn capture_original_shadow_settings(
    mut cmds: Commands,
    dir: Query<
        (Entity, &DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalShadowEnabled>),
    >,
    pt: Query<(Entity, &PointLight), (Added<PointLight>, Without<OriginalShadowEnabled>)>,
    sp: Query<(Entity, &SpotLight), (Added<SpotLight>, Without<OriginalShadowEnabled>)>,
) {
    for (entity, light) in &dir {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
    for (entity, light) in &pt {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
    for (entity, light) in &sp {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
}

pub(super) fn apply_shadow_toggle(
    toggles: Res<DisplayToggles>,
    mut dir: Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
    mut pt: Query<(&mut PointLight, &OriginalShadowEnabled)>,
    mut sp: Query<(&mut SpotLight, &OriginalShadowEnabled)>,
) {
    for (mut light, authored) in &mut dir {
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored.0,
        );
    }
    for (mut light, authored) in &mut pt {
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored.0,
        );
    }
    for (mut light, authored) in &mut sp {
        set_shadow_enabled(
            &mut light.shadow_maps_enabled,
            toggles.renderer.shadows && authored.0,
        );
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
