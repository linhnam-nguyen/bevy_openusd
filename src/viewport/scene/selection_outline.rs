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
const MAX_PRESENTATION_ENTITIES_PER_UPDATE: usize = 256;

/// Marks outline components owned by the selection presentation path.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionOutline;

#[derive(Debug, Clone, PartialEq)]
struct OutlineWorkKey {
    selection_revision: u64,
    scene_revision: u64,
    projection_generation: Option<u64>,
    boundary: (bool, ColorRgb8),
}

#[derive(Debug)]
struct PendingOutlineWork {
    key: OutlineWorkKey,
    desired: HashSet<Entity>,
    added: HashSet<Entity>,
    removed: Vec<Entity>,
    updated: Vec<Entity>,
    removed_offset: usize,
    updated_offset: usize,
}

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SelectionOutlineState {
    /// The completed desired set from the last fully reconciled work item.
    entities: HashSet<Entity>,
    /// The entities that currently have outline work physically applied. A
    /// bounded work item can be interrupted after a prefix has been queued,
    /// so cancellation must reconcile from this set rather than `entities`.
    applied_entities: HashSet<Entity>,
    last_boundary: Option<(bool, ColorRgb8)>,
    last_projection_generation: Option<u64>,
    last_selection_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    pending: Option<PendingOutlineWork>,
    pub(in crate::viewport) last_added: usize,
    pub(in crate::viewport) last_removed: usize,
    pub(in crate::viewport) last_updated: usize,
}

impl SelectionOutlineState {
    pub(in crate::viewport) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
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
    let key = OutlineWorkKey {
        selection_revision: selection.revision(),
        scene_revision: scene_index.revision(),
        projection_generation,
        boundary,
    };
    state.last_added = 0;
    state.last_removed = 0;
    state.last_updated = 0;
    if state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.key != key)
    {
        state.pending = None;
    }
    if state.pending.is_some() {
        apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
        return;
    }
    if state.last_selection_revision == Some(key.selection_revision)
        && state.last_scene_revision == Some(key.scene_revision)
        && state.last_projection_generation == key.projection_generation
        && state.last_boundary == Some(key.boundary)
    {
        return;
    }

    let boundary_changed = state.last_boundary != Some(key.boundary);
    let projection_changed = state.last_projection_generation != key.projection_generation;
    let can_use_projection_delta =
        !boundary_changed && projection_changed && projection.is_some() && key.boundary.0;

    let (added, removed, desired) = if can_use_projection_delta {
        let projection = projection.as_ref().expect("checked above");
        let added = projection
            .added_renderables()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let removed = projection
            .removed_renderables()
            .intersection(&state.applied_entities)
            .copied()
            .collect::<Vec<_>>();
        (added, removed, HashSet::new())
    } else {
        let desired = if key.boundary.0 {
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
        let added = desired
            .difference(&state.applied_entities)
            .copied()
            .collect::<Vec<_>>();
        let removed = state
            .applied_entities
            .difference(&desired)
            .copied()
            .collect::<Vec<_>>();
        (added, removed, desired)
    };
    let mut to_update = if boundary_changed {
        desired.iter().copied().collect::<Vec<_>>()
    } else {
        added.clone()
    };
    to_update.sort_unstable();
    let mut removed = removed;
    removed.sort_unstable();
    state.pending = Some(PendingOutlineWork {
        key,
        desired,
        added: added.into_iter().collect(),
        removed,
        updated: to_update,
        removed_offset: 0,
        updated_offset: 0,
    });
    apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
}

fn apply_pending_outline_work(
    state: &mut SelectionOutlineState,
    commands: &mut Commands,
    owned_outlines: &Query<(), With<SelectionOutline>>,
) {
    let Some(mut work) = state.pending.take() else {
        return;
    };
    let outline = OutlineVolume {
        visible: work.key.boundary.0,
        width: SELECTION_OUTLINE_WIDTH,
        colour: color_from_rgb8(work.key.boundary.1),
    };
    let mut budget = MAX_PRESENTATION_ENTITIES_PER_UPDATE;
    while budget > 0 && work.removed_offset < work.removed.len() {
        let entity = work.removed[work.removed_offset];
        work.removed_offset += 1;
        budget -= 1;
        if owned_outlines.get(entity).is_ok() {
            commands
                .entity(entity)
                .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
        }
        state.applied_entities.remove(&entity);
        state.last_removed += 1;
    }
    while budget > 0 && work.updated_offset < work.updated.len() {
        let entity = work.updated[work.updated_offset];
        work.updated_offset += 1;
        budget -= 1;
        commands.entity(entity).insert((
            SelectionOutline,
            outline.clone(),
            OutlineStencil::default(),
        ));
        state.applied_entities.insert(entity);
        state.last_updated += 1;
        if work.added.contains(&entity) {
            state.last_added += 1;
        }
    }
    if work.removed_offset < work.removed.len() || work.updated_offset < work.updated.len() {
        state.pending = Some(work);
        return;
    }
    if !work.desired.is_empty() || !work.key.boundary.0 {
        state.entities = work.desired.clone();
        state.applied_entities = work.desired;
    } else {
        state.entities = state.applied_entities.clone();
    }
    state.last_boundary = Some(work.key.boundary);
    state.last_projection_generation = work.key.projection_generation;
    state.last_selection_revision = Some(work.key.selection_revision);
    state.last_scene_revision = Some(work.key.scene_revision);
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
