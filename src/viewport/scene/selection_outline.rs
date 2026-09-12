//! Renderer-owned selection silhouettes.
//!
//! Selection identity remains a [`SceneAnchor`]. This module resolves those
//! anchors through the session-local index and attaches only transient outline
//! components to the current projected mesh entities.

use std::collections::HashSet;

use bevy::prelude::*;
#[cfg(test)]
use bevy_mod_outline::OutlineVolume;
use viewport_protocol::ColorRgb8;
#[cfg(test)]
use viewport_protocol::SelectionPresentationSettings;

#[path = "selection_outline_apply.rs"]
mod apply;
#[path = "selection_outline_tree.rs"]
mod tree;
use apply::apply_pending_outline_work;
pub(in crate::viewport) use tree::collect_mesh_descendants;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::{
    CoarseSelectionPresentation, SelectedRenderableProjection, SelectedTargets,
    SelectionPresentationPolicy,
};

use super::entity_order::EntityOrder;

const SELECTION_OUTLINE_WIDTH: f32 = 3.0;
const MAX_PRESENTATION_ENTITIES_PER_UPDATE: usize = 256;

/// Marks outline components owned by the selection presentation path.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionOutline;

#[derive(Debug, Clone, PartialEq)]
struct OutlineWorkKey {
    selection_revision: u64,
    /// A cached projection already owns target resolution. Global scene churn
    /// is irrelevant until that authority is absent.
    scene_revision: Option<u64>,
    projection_generation: Option<u64>,
    boundary: (bool, ColorRgb8),
    coarse: bool,
}

#[derive(Debug)]
struct PendingOutlineWork {
    key: OutlineWorkKey,
    added: HashSet<Entity>,
    removed: Vec<Entity>,
    updated: Vec<Entity>,
    removed_offset: usize,
    updated_offset: usize,
    reconcile_all: bool,
    reconcile_phase: u8,
    reconcile_offset: usize,
}

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SelectionOutlineState {
    /// The desired set is updated immediately from projection deltas and is
    /// never discarded when an in-flight work item is superseded.
    desired_entities: HashSet<Entity>,
    desired_order: EntityOrder,
    /// The entities that currently have outline work physically applied. A
    /// bounded work item can be interrupted after a prefix has been queued,
    /// so cancellation must reconcile from this set rather than `entities`.
    applied_entities: HashSet<Entity>,
    applied_order: EntityOrder,
    last_boundary: Option<(bool, ColorRgb8)>,
    last_projection_generation: Option<u64>,
    last_selection_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    last_coarse: Option<bool>,
    coarse_roots: HashSet<Entity>,
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
    policy: Option<Res<SelectionPresentationPolicy>>,
    projection: Res<SelectedRenderableProjection>,
    mut state: ResMut<SelectionOutlineState>,
    mut commands: Commands,
    owned_outlines: Query<(), With<SelectionOutline>>,
) {
    let presentation = settings.selection();
    let boundary = (presentation.boundary_enabled, presentation.boundary_color);
    let coarse = policy.as_deref().is_some_and(|policy| {
        policy.uses_coarse(projection.renderables().len())
    });
    let projection_generation = Some(projection.generation());
    let key = OutlineWorkKey {
        selection_revision: selection.revision(),
        scene_revision: None,
        projection_generation,
        boundary,
        coarse,
    };
    state.last_added = 0;
    state.last_removed = 0;
    state.last_updated = 0;

    if coarse {
        let desired_coarse_roots = selection
            .0
            .targets
            .iter()
            .filter_map(|target| scene_index.resolve(target))
            .collect::<HashSet<_>>();
        for root in state
            .coarse_roots
            .difference(&desired_coarse_roots)
            .copied()
            .collect::<Vec<_>>()
        {
            commands
                .entity(root)
                .remove::<CoarseSelectionPresentation>();
        }
        for root in desired_coarse_roots
            .difference(&state.coarse_roots)
            .copied()
        {
            commands.entity(root).insert(CoarseSelectionPresentation);
        }
        state.coarse_roots = desired_coarse_roots;
    } else if !state.coarse_roots.is_empty() {
        for root in state.coarse_roots.drain() {
            commands
                .entity(root)
                .remove::<CoarseSelectionPresentation>();
        }
    }

    if coarse {
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.key == key)
        {
            apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
            return;
        }
        if state.pending.is_none()
            && state.last_selection_revision == Some(key.selection_revision)
            && state.last_scene_revision == key.scene_revision
            && state.last_projection_generation == key.projection_generation
            && state.last_boundary == Some(key.boundary)
            && state.last_coarse == Some(key.coarse)
        {
            return;
        }
        state.desired_entities.clear();
        state.desired_order.clear();
        state.pending = Some(PendingOutlineWork {
            key,
            added: HashSet::new(),
            removed: state.applied_entities.iter().copied().collect(),
            updated: Vec::new(),
            removed_offset: 0,
            updated_offset: 0,
            reconcile_all: true,
            reconcile_phase: 0,
            reconcile_offset: 0,
        });
        apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
        return;
    }

    let superseded_pending = state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.key != key);
    if state.pending.is_some() && !superseded_pending {
        apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
        return;
    }
    if state.last_selection_revision == Some(key.selection_revision)
        && state.last_scene_revision == key.scene_revision
        && state.last_projection_generation == key.projection_generation
        && state.last_boundary == Some(key.boundary)
        && state.last_coarse == Some(key.coarse)
    {
        return;
    }

    let boundary_changed = state.last_boundary != Some(key.boundary);
    let projection_changed = state.last_projection_generation != key.projection_generation;
    // Selection replacement remains bounded because projection membership is
    // the sole desired-set authority. Its added/removed deltas are themselves
    // produced by the resumable projection cursor.
    let can_use_projection_delta = projection_changed;

    let (added, removed) = if can_use_projection_delta {
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
        for entity in added.iter().copied() {
            if state.desired_entities.insert(entity) {
                debug_assert!(state.desired_order.insert(entity));
            }
        }
        for entity in removed.iter().copied() {
            if state.desired_entities.remove(&entity) {
                debug_assert!(state.desired_order.remove(entity));
            }
        }
        (added, removed)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut to_update = added.clone();
    to_update.sort_unstable();
    let mut removed = removed;
    removed.sort_unstable();
    state.pending = Some(PendingOutlineWork {
        key,
        added: added.into_iter().collect(),
        removed,
        updated: to_update,
        removed_offset: 0,
        updated_offset: 0,
        reconcile_all: boundary_changed,
        // EntityOrder removes with swap_remove. A superseding job may have
        // changed the dense order behind an old numeric cursor, so every new
        // reconciliation starts from the beginning of the current order.
        reconcile_phase: 0,
        reconcile_offset: 0,
    });
    apply_pending_outline_work(&mut state, &mut commands, &owned_outlines);
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
