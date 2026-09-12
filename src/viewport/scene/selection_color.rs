//! Renderer-owned selection and hover color overrides.
//!
//! Selection and hover colors are temporary material rebinds on projected mesh
//! entities. The authoritative USD material is retained in
//! [`SelectionBaseMaterial`] and restored when no presentation owns the mesh.

use std::collections::{HashMap, HashSet};

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use viewport_protocol::{ColorRgb8, SceneAnchor};

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::{
    SelectedRenderableProjection, SelectedTargets, SelectionPresentationPolicy,
};

use super::entity_order::EntityOrder;
use super::selection_hover::HoveredTarget;
use super::selection_outline::collect_mesh_descendants;

#[path = "selection_color_apply.rs"]
mod apply;
use apply::apply_pending_color_work;

/// Marks a mesh whose material is currently owned by selection presentation.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport) struct SelectionColorOverride;

/// Material route underneath [`SelectionColorOverride`].
#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct SelectionBaseMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

const MAX_PRESENTATION_ENTITIES_PER_UPDATE: usize = 256;
type PresentationKey = (bool, ColorRgb8, bool, ColorRgb8, Option<SceneAnchor>, bool);

#[derive(Debug, Clone, PartialEq)]
struct ColorWorkKey {
    selection_revision: u64,
    /// A cached projection already owns target resolution. Global scene churn
    /// is irrelevant until that authority is absent.
    scene_revision: Option<u64>,
    projection_generation: Option<u64>,
    presentation: PresentationKey,
}

#[derive(Debug, Clone)]
struct PendingColorWork {
    key: ColorWorkKey,
    affected: Vec<Entity>,
    offset: usize,
    reconcile_all: bool,
    reconcile_phase: u8,
    reconcile_offset: usize,
}

#[derive(Resource, Debug, Clone)]
pub(in crate::viewport) struct SelectionColorMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Default, Clone)]
pub(in crate::viewport) struct SelectionColorOverrideState {
    last_presentation: Option<PresentationKey>,
    last_selection_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    selected_meshes: HashSet<Entity>,
    hovered_meshes: HashSet<Entity>,
    selected_order: EntityOrder,
    hovered_order: EntityOrder,
    applied_owners: HashMap<Entity, PresentationOwner>,
    applied_order: EntityOrder,
    last_projection_generation: Option<u64>,
    pending: Option<PendingColorWork>,
    pub(in crate::viewport) last_affected_entities: usize,
}

impl SelectionColorOverrideState {
    pub(in crate::viewport) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

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
/// and presentation settings.
/// It never edits the USD stage or creates a material per target.
pub(in crate::viewport) fn sync_selection_color_overrides(
    selection: Res<SelectedTargets>,
    settings: Res<ViewerSettingsState>,
    scene_index: Res<SceneAnchorIndex>,
    policy: Option<Res<SelectionPresentationPolicy>>,
    hovered_target: Res<HoveredTarget>,
    projection: Res<SelectedRenderableProjection>,
    color_material: Option<Res<SelectionColorMaterial>>,
    hover_material: Option<Res<HoverColorMaterial>>,
    mut state: ResMut<SelectionColorOverrideState>,
    mut commands: Commands,
    mut material_assets: ResMut<Assets<StandardMaterial>>,
    mesh_hierarchy: Query<(Option<&Mesh3d>, Option<&Children>)>,
    mut meshes: Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&SelectionBaseMaterial>,
        Option<&SelectionColorOverride>,
    )>,
) {
    let (Some(color_material), Some(hover_material)) = (color_material, hover_material) else {
        return;
    };
    let presentation = settings.selection();
    let coarse = policy.as_deref().is_some_and(|policy| {
        policy.uses_coarse(projection.renderables().len())
    });
    let presentation_key: PresentationKey = (
        presentation.color_change_enabled,
        presentation.selection_color,
        presentation.hover_color_change_enabled,
        presentation.hover_color,
        hovered_target.anchor.clone(),
        coarse,
    );
    let projection_generation = Some(projection.generation());
    let key = ColorWorkKey {
        selection_revision: selection.revision(),
        scene_revision: None,
        projection_generation,
        presentation: presentation_key,
    };
    state.last_affected_entities = 0;
    let superseded_pending = state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.key != key);
    if state.pending.is_some() && !superseded_pending {
        apply_pending_color_work(
            &mut state,
            &mut commands,
            &mut meshes,
            &color_material.0,
            &hover_material.0,
        );
        return;
    }
    if !superseded_pending
        && state.last_selection_revision == Some(key.selection_revision)
        && state.last_scene_revision == key.scene_revision
        && state.last_projection_generation == key.projection_generation
        && state.last_presentation.as_ref() == Some(&key.presentation)
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

    let projection_changed = state.last_projection_generation != projection_generation;
    let mut full_reconcile = false;
    if coarse {
        state.selected_meshes.clear();
        state.selected_order.clear();
        full_reconcile = true;
    } else if projection_changed {
        for added in projection.added_renderables() {
            if state.selected_meshes.insert(*added) {
                debug_assert!(state.selected_order.insert(*added));
            }
        }
        for removed in projection.removed_renderables() {
            if state.selected_meshes.remove(removed) {
                debug_assert!(state.selected_order.remove(*removed));
            }
        }
    }
    let mut hovered_meshes = HashSet::new();
    if !coarse
        && presentation.hover_color_change_enabled
        && let Some(target) = hovered_target.anchor.as_ref()
        && let Some(entity) = scene_index.resolve(target)
    {
        collect_mesh_descendants(entity, &mesh_hierarchy, &mut hovered_meshes);
    }

    if hovered_meshes != state.hovered_meshes {
        state.hovered_order.clear();
        for entity in hovered_meshes.iter().copied() {
            debug_assert!(state.hovered_order.insert(entity));
        }
        state.hovered_meshes = hovered_meshes;
        full_reconcile = true;
    }
    let mut affected = HashSet::new();
    if projection_changed {
        affected.extend(projection.added_renderables().iter().copied());
        affected.extend(projection.removed_renderables().iter().copied());
    }
    full_reconcile |= selection_color_changed || hover_color_changed;
    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable();
    state.pending = Some(PendingColorWork {
        key,
        affected,
        offset: 0,
        reconcile_all: full_reconcile,
        // EntityOrder uses swap_remove, so a cursor from an older dense order
        // cannot be resumed safely after a superseding projection/presentation
        // change. Reconcile the current order from zero.
        reconcile_phase: 0,
        reconcile_offset: 0,
    });
    apply_pending_color_work(
        &mut state,
        &mut commands,
        &mut meshes,
        &color_material.0,
        &hover_material.0,
    );
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
