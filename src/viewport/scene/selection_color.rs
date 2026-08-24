//! Renderer-owned selection and hover color overrides.
//!
//! Selection and hover colors are temporary material rebinds on projected mesh
//! entities. The authoritative USD material is retained in
//! [`SelectionBaseMaterial`] and restored when no presentation owns the mesh.

use std::collections::HashSet;

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use viewport_protocol::{ColorRgb8, SceneAnchor, SelectionReadModel};

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::SelectedTargets;

use super::selection_hover::HoveredTarget;
use super::selection_outline::collect_mesh_descendants;

/// Marks a mesh whose material is currently owned by selection presentation.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport) struct SelectionColorOverride;

/// Material route underneath [`SelectionColorOverride`].
#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct SelectionBaseMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Clone)]
pub(in crate::viewport) struct SelectionColorMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Default, Clone)]
pub(super) struct SelectionColorOverrideState {
    last_presentation: Option<(bool, ColorRgb8, bool, ColorRgb8, Option<SceneAnchor>)>,
    last_selection: Option<SelectionReadModel>,
    last_scene_revision: Option<u64>,
    selected_meshes: HashSet<Entity>,
    hovered_meshes: HashSet<Entity>,
}

#[derive(Resource, Debug, Clone)]
pub(in crate::viewport) struct HoverColorMaterial(pub(in crate::viewport) Handle<StandardMaterial>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentationOwner {
    Selection,
    Hover,
}

fn presentation_owner(selected: bool, hovered: bool) -> Option<PresentationOwner> {
    if selected {
        Some(PresentationOwner::Selection)
    } else if hovered {
        Some(PresentationOwner::Hover)
    } else {
        None
    }
}

pub(super) fn init_selection_color_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Option<Res<SelectionColorMaterial>>,
    existing_hover: Option<Res<HoverColorMaterial>>,
) {
    if existing.is_none() {
        commands.insert_resource(SelectionColorMaterial(materials.add(StandardMaterial {
            perceptual_roughness: 1.0,
            ..default()
        })));
    }
    if existing_hover.is_none() {
        commands.insert_resource(HoverColorMaterial(materials.add(StandardMaterial {
            perceptual_roughness: 1.0,
            ..default()
        })));
    }
}

/// Rebinds selected or hovered projected meshes to shared presentation materials.
///
/// The system is change-gated by logical selection, scene-anchor resolution,
/// selection settings, and material changes on already-overridden entities.
/// It never edits the USD stage or creates a material per target.
pub(super) fn sync_selection_color_overrides(
    selection: Res<SelectedTargets>,
    settings: Res<ViewerSettingsState>,
    scene_index: Res<SceneAnchorIndex>,
    hovered_target: Res<HoveredTarget>,
    color_material: Option<Res<SelectionColorMaterial>>,
    hover_material: Option<Res<HoverColorMaterial>>,
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
            Entity,
            (
                With<SelectionColorOverride>,
                Changed<MeshMaterial3d<StandardMaterial>>,
            ),
        >,
    )>,
) {
    let (Some(color_material), Some(hover_material)) = (color_material, hover_material) else {
        return;
    };
    let presentation = settings.selection();
    let presentation_key = (
        presentation.color_change_enabled,
        presentation.selection_color,
        presentation.hover_color_change_enabled,
        presentation.hover_color,
        hovered_target.anchor.clone(),
    );
    let changed_owned_entities: Vec<Entity> = meshes.p1().iter().collect();
    let material_changed = !changed_owned_entities.is_empty();
    let selection_changed = state.last_selection.as_ref() != Some(&selection.0);
    let scene_changed = state.last_scene_revision != Some(scene_index.revision());
    if !selection_changed
        && !scene_changed
        && state.last_presentation.as_ref() == Some(&presentation_key)
        && !material_changed
    {
        return;
    }

    let selection_color_changed = state
        .last_presentation
        .as_ref()
        .is_none_or(|last| last.1 != presentation.selection_color);
    let hover_color_changed = state
        .last_presentation
        .as_ref()
        .is_none_or(|last| last.3 != presentation.hover_color);
    if selection_color_changed
        && let Some(mut material) = material_assets.get_mut(&color_material.0)
    {
        material.base_color = color_from_rgb8(presentation.selection_color);
    }
    if hover_color_changed && let Some(mut material) = material_assets.get_mut(&hover_material.0) {
        material.base_color = color_from_rgb8(presentation.hover_color);
    }

    let sets_changed = selection_changed
        || scene_changed
        || state.last_presentation.as_ref().is_none_or(|last| {
            last.0 != presentation.color_change_enabled
                || last.2 != presentation.hover_color_change_enabled
                || last.4.as_ref() != hovered_target.anchor.as_ref()
        });
    let previous_selected_meshes = std::mem::take(&mut state.selected_meshes);
    let previous_hovered_meshes = std::mem::take(&mut state.hovered_meshes);
    let (selected_meshes, hovered_meshes) = if sets_changed {
        let mut selected_meshes = HashSet::new();
        if presentation.color_change_enabled {
            for target in &selection.0.targets {
                let Some(entity) = scene_index.resolve(target) else {
                    continue;
                };
                collect_mesh_descendants(entity, &mesh_hierarchy, &mut selected_meshes);
            }
        }

        let mut hovered_meshes = HashSet::new();
        if presentation.hover_color_change_enabled
            && let Some(target) = hovered_target.anchor.as_ref()
            && let Some(entity) = scene_index.resolve(target)
        {
            collect_mesh_descendants(entity, &mesh_hierarchy, &mut hovered_meshes);
        }
        (selected_meshes, hovered_meshes)
    } else {
        (
            previous_selected_meshes.clone(),
            previous_hovered_meshes.clone(),
        )
    };

    let mut affected = HashSet::new();
    if sets_changed {
        affected.extend(previous_selected_meshes.iter().copied());
        affected.extend(previous_hovered_meshes.iter().copied());
        affected.extend(selected_meshes.iter().copied());
        affected.extend(hovered_meshes.iter().copied());
    }
    if selection_color_changed {
        affected.extend(selected_meshes.iter().copied());
    }
    if hover_color_changed {
        affected.extend(hovered_meshes.iter().copied());
    }
    affected.extend(changed_owned_entities);

    let selection_handle = &color_material.0;
    let hover_handle = &hover_material.0;
    let mut meshes = meshes.p0();
    for entity in affected {
        let Ok((_, mut material, base, marker)) = meshes.get_mut(entity) else {
            continue;
        };
        let desired_handle = match presentation_owner(
            selected_meshes.contains(&entity),
            hovered_meshes.contains(&entity),
        ) {
            Some(PresentationOwner::Selection) => Some(selection_handle),
            Some(PresentationOwner::Hover) => Some(hover_handle),
            None => None,
        };

        if let Some(desired_handle) = desired_handle {
            if let (Some(base), Some(_marker)) = (base, marker)
                && material.0 != *selection_handle
                && material.0 != *hover_handle
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
            if material.0 != *desired_handle {
                material.0 = desired_handle.clone();
            }
        } else if let (Some(base), Some(_marker)) = (base, marker) {
            material.0 = base.0.clone();
            commands
                .entity(entity)
                .remove::<(SelectionColorOverride, SelectionBaseMaterial)>();
        }
    }

    state.selected_meshes = selected_meshes;
    state.hovered_meshes = hovered_meshes;
    state.last_selection = Some(selection.0.clone());
    state.last_scene_revision = Some(scene_index.revision());
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

    #[test]
    fn selection_color_has_priority_over_hover_color() {
        assert_eq!(
            presentation_owner(true, true),
            Some(PresentationOwner::Selection)
        );
        assert_eq!(
            presentation_owner(false, true),
            Some(PresentationOwner::Hover)
        );
    }
}
