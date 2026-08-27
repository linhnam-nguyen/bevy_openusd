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
use crate::viewport::scene::{SelectedRenderableProjection, SelectedTargets};

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

const MAX_PRESENTATION_ENTITIES_PER_UPDATE: usize = 256;
type PresentationKey = (bool, ColorRgb8, bool, ColorRgb8, Option<SceneAnchor>);

#[derive(Debug, Clone, PartialEq)]
struct ColorWorkKey {
    selection_revision: u64,
    scene_revision: u64,
    projection_generation: Option<u64>,
    presentation: PresentationKey,
}

#[derive(Debug, Clone)]
struct PendingColorWork {
    key: ColorWorkKey,
    selected_meshes: HashSet<Entity>,
    hovered_meshes: HashSet<Entity>,
    affected: Vec<Entity>,
    offset: usize,
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
    applied_owners: HashMap<Entity, PresentationOwner>,
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
    hovered_target: Res<HoveredTarget>,
    projection: Option<Res<SelectedRenderableProjection>>,
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
    let presentation_key: PresentationKey = (
        presentation.color_change_enabled,
        presentation.selection_color,
        presentation.hover_color_change_enabled,
        presentation.hover_color,
        hovered_target.anchor.clone(),
    );
    let projection_generation = projection
        .as_ref()
        .map(|projection| projection.generation());
    let key = ColorWorkKey {
        selection_revision: selection.revision(),
        scene_revision: scene_index.revision(),
        projection_generation,
        presentation: presentation_key,
    };
    state.last_affected_entities = 0;
    let superseded_pending = state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.key != key);
    let superseded_applied = if superseded_pending {
        state.pending = None;
        state.applied_owners.keys().copied().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if state.pending.is_some() {
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
        && state.last_scene_revision == Some(key.scene_revision)
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
    let selected_meshes = if presentation.color_change_enabled {
        projection.as_ref().map_or_else(
            || {
                let mut selected_meshes = HashSet::new();
                for target in &selection.0.targets {
                    let Some(entity) = scene_index.resolve(target) else {
                        continue;
                    };
                    collect_mesh_descendants(entity, &mesh_hierarchy, &mut selected_meshes);
                }
                selected_meshes
            },
            |projection| projection.renderables().clone(),
        )
    } else {
        HashSet::new()
    };
    let mut hovered_meshes = HashSet::new();
    if presentation.hover_color_change_enabled
        && let Some(target) = hovered_target.anchor.as_ref()
        && let Some(entity) = scene_index.resolve(target)
    {
        collect_mesh_descendants(entity, &mesh_hierarchy, &mut hovered_meshes);
    }

    let previous_selected_meshes = &state.selected_meshes;
    let previous_hovered_meshes = &state.hovered_meshes;
    let mut affected = HashSet::new();
    let can_use_projection_delta = projection_changed
        && projection.is_some()
        && state
            .last_presentation
            .as_ref()
            .is_some_and(|last| last.0 == presentation.color_change_enabled);
    if can_use_projection_delta {
        let projection = projection.as_ref().expect("projection is present");
        affected.extend(projection.added_renderables().iter().copied());
        affected.extend(projection.removed_renderables().iter().copied());
    } else if previous_selected_meshes != &selected_meshes {
        affected.extend(
            previous_selected_meshes
                .symmetric_difference(&selected_meshes)
                .copied(),
        );
    }
    if previous_hovered_meshes != &hovered_meshes {
        affected.extend(
            previous_hovered_meshes
                .symmetric_difference(&hovered_meshes)
                .copied(),
        );
    }
    if selection_color_changed {
        affected.extend(selected_meshes.iter().copied());
    }
    if hover_color_changed {
        affected.extend(hovered_meshes.iter().copied());
    }
    affected.extend(superseded_applied);
    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable();
    state.pending = Some(PendingColorWork {
        key,
        selected_meshes,
        hovered_meshes,
        affected,
        offset: 0,
    });
    apply_pending_color_work(
        &mut state,
        &mut commands,
        &mut meshes,
        &color_material.0,
        &hover_material.0,
    );
}

fn apply_pending_color_work(
    state: &mut SelectionColorOverrideState,
    commands: &mut Commands,
    meshes: &mut Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&SelectionBaseMaterial>,
        Option<&SelectionColorOverride>,
    )>,
    selection_handle: &Handle<StandardMaterial>,
    hover_handle: &Handle<StandardMaterial>,
) {
    let Some(mut work) = state.pending.take() else {
        return;
    };
    let start = work.offset;
    let end = (start + MAX_PRESENTATION_ENTITIES_PER_UPDATE).min(work.affected.len());
    for entity in &work.affected[start..end] {
        let entity = *entity;
        let Ok((_, mut material, base, marker)) = meshes.get_mut(entity) else {
            state.applied_owners.remove(&entity);
            continue;
        };
        let desired_owner = presentation_owner(
            work.selected_meshes.contains(&entity),
            work.hovered_meshes.contains(&entity),
        );
        if let Some(desired_owner) = desired_owner {
            let desired_handle = match desired_owner {
                PresentationOwner::Selection => selection_handle,
                PresentationOwner::Hover => hover_handle,
            };
            if let (Some(base), Some(_marker)) = (base, marker)
                && material.0 != *selection_handle
                && material.0 != *hover_handle
                && material.0 != base.0
            {
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
            state.applied_owners.insert(entity, desired_owner);
        } else if let (Some(base), Some(_marker)) = (base, marker) {
            material.0 = base.0.clone();
            commands
                .entity(entity)
                .remove::<(SelectionColorOverride, SelectionBaseMaterial)>();
            state.applied_owners.remove(&entity);
        } else {
            state.applied_owners.remove(&entity);
        }
    }
    work.offset = end;
    state.last_affected_entities = end - start;
    if work.offset < work.affected.len() {
        state.pending = Some(work);
        return;
    }
    state.selected_meshes = work.selected_meshes;
    state.hovered_meshes = work.hovered_meshes;
    state.last_selection_revision = Some(work.key.selection_revision);
    state.last_scene_revision = Some(work.key.scene_revision);
    state.last_projection_generation = work.key.projection_generation;
    state.last_presentation = Some(work.key.presentation);
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
