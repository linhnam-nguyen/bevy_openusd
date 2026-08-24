//! Renderer-owned selection color overrides.
//!
//! Selection color is a temporary material rebind on projected mesh entities.
//! The authoritative USD material is retained in [`SelectionBaseMaterial`] and
//! restored when the target is no longer selected or the feature is disabled.

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use viewport_protocol::ColorRgb8;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::SelectedTargets;

use super::selection_outline::collect_mesh_descendants;

/// Marks a mesh whose material is currently owned by selection presentation.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelectionColorOverride;

/// Material route underneath [`SelectionColorOverride`].
#[derive(Component, Debug, Clone)]
pub(super) struct SelectionBaseMaterial(pub(super) Handle<StandardMaterial>);

#[derive(Resource, Debug, Clone)]
pub(super) struct SelectionColorMaterial(pub(super) Handle<StandardMaterial>);

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct SelectionColorOverrideState {
    last_presentation: Option<(bool, ColorRgb8)>,
}

pub(super) fn init_selection_color_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Option<Res<SelectionColorMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    commands.insert_resource(SelectionColorMaterial(materials.add(StandardMaterial {
        perceptual_roughness: 1.0,
        ..default()
    })));
}

/// Rebinds selected projected meshes to one shared selection material.
///
/// The system is change-gated by logical selection, scene-anchor resolution,
/// selection settings, and material changes on already-overridden entities.
/// It never edits the USD stage or creates a material per target.
pub(super) fn sync_selection_color_overrides(
    selection: Res<SelectedTargets>,
    settings: Res<ViewerSettingsState>,
    scene_index: Res<SceneAnchorIndex>,
    color_material: Option<Res<SelectionColorMaterial>>,
    mut state: ResMut<SelectionColorOverrideState>,
    mut commands: Commands,
    mut material_assets: ResMut<Assets<StandardMaterial>>,
    mesh_hierarchy: Query<(Option<&Mesh3d>, Option<&Children>)>,
    mut meshes: ParamSet<(
        Query<(
            Entity,
            &mut MeshMaterial3d<StandardMaterial>,
            Option<&SelectionBaseMaterial>,
            Option<&SelectionColorOverride>,
        )>,
        Query<
            (),
            (
                With<SelectionColorOverride>,
                Changed<MeshMaterial3d<StandardMaterial>>,
            ),
        >,
    )>,
) {
    let Some(color_material) = color_material else {
        return;
    };
    let presentation = settings.selection();
    let presentation_key = (
        presentation.color_change_enabled,
        presentation.selection_color,
    );
    let material_changed = meshes.p1().iter().next().is_some();
    if !selection.is_changed()
        && !scene_index.is_changed()
        && state.last_presentation == Some(presentation_key)
        && !material_changed
    {
        return;
    }

    if let Some(mut material) = material_assets.get_mut(&color_material.0) {
        material.base_color = color_from_rgb8(presentation.selection_color);
    }

    let mut desired = std::collections::HashSet::new();
    if presentation.color_change_enabled {
        for target in &selection.0.targets {
            let Some(entity) = scene_index.resolve(target) else {
                continue;
            };
            collect_mesh_descendants(entity, &mesh_hierarchy, &mut desired);
        }
    }

    let color_handle = &color_material.0;
    let mut meshes = meshes.p0();
    for (entity, mut material, base, marker) in &mut meshes {
        if desired.contains(&entity) {
            if let (Some(base), Some(_marker)) = (base, marker)
                && material.0 != *color_handle
                && material.0 != base.0
            {
                // A projection route changed while the temporary override was
                // active. Keep the new route as the restoration target.
                commands
                    .entity(entity)
                    .insert(SelectionBaseMaterial(material.0.clone()));
            }

            if marker.is_none() || base.is_none() {
                commands.entity(entity).insert((
                    SelectionColorOverride,
                    SelectionBaseMaterial(material.0.clone()),
                ));
            }
            if material.0 != *color_handle {
                material.0 = color_handle.clone();
            }
        } else if let (Some(base), Some(_marker)) = (base, marker) {
            material.0 = base.0.clone();
            commands
                .entity(entity)
                .remove::<(SelectionColorOverride, SelectionBaseMaterial)>();
        }
    }

    state.last_presentation = Some(presentation_key);
}

pub(super) fn color_from_rgb8(color: ColorRgb8) -> Color {
    Color::srgba(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        1.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewport_protocol::SelectionPresentationSettings;

    #[test]
    fn selection_color_preserves_rgb_channels() {
        let color = color_from_rgb8(ColorRgb8::new(0x34, 0xA0, 0xF2));
        assert_eq!(color.to_srgba().to_u8_array(), [0x34, 0xA0, 0xF2, 0xFF]);
    }

    #[test]
    fn default_selection_color_override_is_disabled_but_has_documented_color() {
        let settings = SelectionPresentationSettings::default();
        assert!(!settings.color_change_enabled);
        assert_eq!(
            color_from_rgb8(settings.selection_color)
                .to_srgba()
                .to_u8_array(),
            [0x38, 0xBD, 0xF8, 0xFF]
        );
    }
}
