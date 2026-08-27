//! Renderer-owned selection silhouettes.
//!
//! Selection identity remains a [`SceneAnchor`]. This module resolves those
//! anchors through the session-local index and attaches only transient outline
//! components to the current projected mesh entities.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_mod_outline::{OutlineStencil, OutlineVolume};
use viewport_protocol::ColorRgb8;
#[cfg(test)]
use viewport_protocol::SelectionPresentationSettings;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::{SelectedRenderableProjection, SelectedTargets};

const SELECTION_OUTLINE_WIDTH: f32 = 3.0;

/// Marks outline components owned by the selection presentation path.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionOutline;

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SelectionOutlineState {
    entities: HashSet<Entity>,
    last_boundary: Option<(bool, ColorRgb8)>,
    last_projection_generation: Option<u64>,
    pub(in crate::viewport) last_added: usize,
    pub(in crate::viewport) last_removed: usize,
    pub(in crate::viewport) last_updated: usize,
}

/// Applies the selected targets' boundary settings to transient projected
/// mesh entities. The scene index is the only bridge from stable selection
/// identity to ECS entities; no Entity is stored in protocol state.
pub(in crate::viewport) fn sync_selection_outlines(
    selection: Res<SelectedTargets>,
    settings: Res<ViewerSettingsState>,
    scene_index: Res<SceneAnchorIndex>,
    projection: Option<Res<SelectedRenderableProjection>>,
    mut state: ResMut<SelectionOutlineState>,
    mut commands: Commands,
    meshes: Query<(Option<&Mesh3d>, Option<&Children>)>,
    owned_outlines: Query<(), With<SelectionOutline>>,
) {
    let presentation = settings.selection();
    let boundary = (presentation.boundary_enabled, presentation.boundary_color);
    let projection_generation = projection
        .as_ref()
        .map(|projection| projection.generation());
    let projection_changed = state.last_projection_generation != projection_generation;
    if !selection.is_changed()
        && !scene_index.is_changed()
        && state.last_boundary == Some(boundary)
        && !projection_changed
    {
        state.last_added = 0;
        state.last_removed = 0;
        state.last_updated = 0;
        return;
    }

    let desired = if presentation.boundary_enabled {
        projection.as_ref().map_or_else(
            || {
                let mut desired = HashSet::new();
                for target in &selection.0.targets {
                    let Some(entity) = scene_index.resolve(target) else {
                        continue;
                    };
                    collect_mesh_descendants(entity, &meshes, &mut desired);
                }
                desired
            },
            |projection| projection.renderables().clone(),
        )
    } else {
        HashSet::new()
    };
    let boundary_changed = state.last_boundary != Some(boundary);
    let (added, removed) = if !boundary_changed
        && projection_changed
        && let Some(projection) = projection.as_ref()
    {
        (
            projection
                .added_renderables()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            projection
                .removed_renderables()
                .intersection(&state.entities)
                .copied()
                .collect::<Vec<_>>(),
        )
    } else {
        (
            desired
                .difference(&state.entities)
                .copied()
                .collect::<Vec<_>>(),
            state
                .entities
                .difference(&desired)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    for entity in &removed {
        if owned_outlines.get(*entity).is_ok() {
            commands
                .entity(*entity)
                .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
        }
    }

    let outline = OutlineVolume {
        visible: presentation.boundary_enabled,
        width: SELECTION_OUTLINE_WIDTH,
        colour: color_from_rgb8(presentation.boundary_color),
    };
    let to_update = if boundary_changed {
        desired.iter().copied().collect::<Vec<_>>()
    } else {
        added.clone()
    };
    let updated_count = to_update.len();
    for entity in to_update {
        commands.entity(entity).insert((
            SelectionOutline,
            outline.clone(),
            OutlineStencil::default(),
        ));
    }

    state.entities = desired;
    state.last_boundary = Some(boundary);
    state.last_projection_generation = projection_generation;
    state.last_added = added.len();
    state.last_removed = removed.len();
    state.last_updated = updated_count;
}

pub(super) fn collect_mesh_descendants(
    root: Entity,
    meshes: &Query<(Option<&Mesh3d>, Option<&Children>)>,
    output: &mut HashSet<Entity>,
) {
    let Ok((mesh, children)) = meshes.get(root) else {
        return;
    };
    if mesh.is_some() {
        output.insert(root);
    }
    if let Some(children) = children {
        for child in children.iter() {
            collect_mesh_descendants(child, meshes, output);
        }
    }
}

fn color_from_rgb8(color: ColorRgb8) -> Color {
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

    #[test]
    fn selection_outline_color_preserves_rgb_channels() {
        let color = color_from_rgb8(ColorRgb8::new(0x12, 0x80, 0xF0));
        assert_eq!(color.to_srgba().to_u8_array(), [0x12, 0x80, 0xF0, 0xFF]);
    }

    #[test]
    fn default_selection_boundary_uses_the_documented_width() {
        let settings = SelectionPresentationSettings::default();
        let outline = OutlineVolume {
            visible: settings.boundary_enabled,
            width: SELECTION_OUTLINE_WIDTH,
            colour: color_from_rgb8(settings.boundary_color),
        };
        assert!(outline.visible);
        assert_eq!(outline.width, 3.0);
    }
}
