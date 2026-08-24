//! Renderer-only six-plane clipping through a compositional Extended StandardMaterial.

use std::collections::{HashMap, HashSet};

use bevy::asset::AssetEvent;
use bevy::asset::AssetPath;
use bevy::ecs::system::SystemParam;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use viewport_protocol::{ColorRgb8, RenderMode, SceneAnchor};

use super::section_box::SectionBoxState;
use super::selection_color::{
    HoverColorMaterial, SelectionBaseMaterial, SelectionColorMaterial, SelectionColorOverride,
};
use super::selection_hover::HoveredTarget;
use super::visualization::{DisplayToggles, OriginalRenderMaterial, UniformRenderMaterial};
use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};

#[path = "section_box_clipping_support.rs"]
mod support;

#[cfg(test)]
#[path = "section_box_clipping_tests.rs"]
mod tests;

use support::{
    apply_composed_route, compose_clipped_route, ensure_clip_material, prune_material_cache,
    selected_meshes,
};

const SHADER_ASSET_PATH: &str = "../../../assets/shaders/section_box_clipping.wgsl";
const PREPASS_SHADER_ASSET_PATH: &str = "../../../assets/shaders/section_box_clipping_prepass.wgsl";

pub(in crate::viewport) fn register_embedded_shaders(app: &mut App) {
    bevy::asset::embedded_asset!(app, "../../../assets/shaders/section_box_clipping.wgsl");
    bevy::asset::embedded_asset!(
        app,
        "../../../assets/shaders/section_box_clipping_prepass.wgsl"
    );
}

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
struct SectionClipUniform {
    world_to_box: Mat4,
    enabled: u32,
    _padding: Vec3,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(in crate::viewport) struct SectionClipExtension {
    #[uniform(100)]
    clip: SectionClipUniform,
}

pub(in crate::viewport) type SectionClipMaterial =
    ExtendedMaterial<StandardMaterial, SectionClipExtension>;

impl MaterialExtension for SectionClipExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(bevy::asset::embedded_path!(SHADER_ASSET_PATH))
                .with_source("embedded"),
        )
    }

    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(bevy::asset::embedded_path!(PREPASS_SHADER_ASSET_PATH))
                .with_source("embedded"),
        )
    }

    fn deferred_fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(bevy::asset::embedded_path!(SHADER_ASSET_PATH))
                .with_source("embedded"),
        )
    }
}

/// The StandardMaterial route that remains visible below the clipping wrapper.
/// It is the composition boundary for the frozen B2/B5 presentation systems.
#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct SectionClipUnderlyingMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SectionClipProjectionState {
    active_entities: HashSet<Entity>,
    selected_meshes: HashSet<Entity>,
    hovered_meshes: HashSet<Entity>,
    material_cache: HashMap<AssetId<StandardMaterial>, Handle<SectionClipMaterial>>,
    last_targets: Option<Vec<SceneAnchor>>,
    last_hovered_anchor: Option<SceneAnchor>,
    last_hover_enabled: Option<bool>,
    last_state_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    last_presentation: Option<SectionClipPresentationKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionClipPresentationKey {
    render_mode: RenderMode,
    selection_color_enabled: bool,
    selection_color: ColorRgb8,
    hover_color_enabled: bool,
    hover_color: ColorRgb8,
    hovered_anchor: Option<SceneAnchor>,
}

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SectionClipDiagnostics {
    pub(in crate::viewport) unsupported_entities: HashSet<Entity>,
    pub(in crate::viewport) missing_material_entities: HashSet<Entity>,
}

#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub(in crate::viewport) struct SectionClipSystemParam<'w, 's> {
    selection_material: Option<Res<'w, SelectionColorMaterial>>,
    hover_material: Option<Res<'w, HoverColorMaterial>>,
    uniform_material: Option<Res<'w, UniformRenderMaterial>>,
    projection: ResMut<'w, SectionClipProjectionState>,
    diagnostics: ResMut<'w, SectionClipDiagnostics>,
    standard_materials: Res<'w, Assets<StandardMaterial>>,
    clip_materials: ResMut<'w, Assets<SectionClipMaterial>>,
    mesh_hierarchy: Query<'w, 's, (Option<&'static Mesh3d>, Option<&'static Children>)>,
    renderables: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static MeshMaterial3d<StandardMaterial>>,
            Option<&'static MeshMaterial3d<SectionClipMaterial>>,
            Option<&'static SectionClipUnderlyingMaterial>,
            Option<&'static OriginalRenderMaterial>,
            Option<&'static SelectionBaseMaterial>,
            Option<&'static SelectionColorOverride>,
        ),
        With<Mesh3d>,
    >,
    changed_clipped: Query<
        'w,
        's,
        Entity,
        (
            With<SectionClipUnderlyingMaterial>,
            Or<(
                Added<SectionClipUnderlyingMaterial>,
                Changed<SectionClipUnderlyingMaterial>,
                Changed<MeshMaterial3d<SectionClipMaterial>>,
            )>,
        ),
    >,
    standard_material_events: Option<MessageReader<'w, 's, AssetEvent<StandardMaterial>>>,
}

/// Applies one aggregate box-space test to selected renderables. Reconciliation
/// is revision/key gated, while material and route updates are bounded to the
/// currently clipped set. StandardMaterial presentation routes remain intact
/// in [`SectionClipUnderlyingMaterial`] instead of being discarded.
#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_section_box_clipping(
    state: Res<SectionBoxState>,
    settings: Res<ViewerSettingsState>,
    toggles: Res<DisplayToggles>,
    hovered_target: Res<HoveredTarget>,
    scene_index: Res<SceneAnchorIndex>,
    mut commands: Commands,
    params: SectionClipSystemParam,
) {
    let SectionClipSystemParam {
        selection_material,
        hover_material,
        uniform_material,
        mut projection,
        mut diagnostics,
        standard_materials,
        mut clip_materials,
        mesh_hierarchy,
        renderables,
        changed_clipped,
        mut standard_material_events,
    } = params;
    let selection = settings.selection();
    let presentation = SectionClipPresentationKey {
        render_mode: toggles.renderer.render_mode,
        selection_color_enabled: selection.color_change_enabled,
        selection_color: selection.selection_color,
        hover_color_enabled: selection.hover_color_change_enabled,
        hover_color: selection.hover_color,
        hovered_anchor: hovered_target.anchor.clone(),
    };
    let state_revision_changed = projection.last_state_revision != Some(state.revision);
    let scene_revision_changed = projection.last_scene_revision != Some(scene_index.revision());
    let selection_set_changed =
        projection.last_targets.as_ref() != Some(&state.targets) || scene_revision_changed;
    let hover_set_changed = projection.last_hovered_anchor.as_ref()
        != hovered_target.anchor.as_ref()
        || projection.last_hover_enabled != Some(selection.hover_color_change_enabled)
        || scene_revision_changed;
    let presentation_changed = projection.last_presentation.as_ref() != Some(&presentation);
    let clipped_route_changed = !changed_clipped.is_empty();
    let material_asset_changed = standard_material_events
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());

    if !state_revision_changed
        && !scene_revision_changed
        && !presentation_changed
        && !clipped_route_changed
        && !material_asset_changed
    {
        return;
    }

    if selection_set_changed {
        projection.selected_meshes = selected_meshes(&state.targets, &scene_index, &mesh_hierarchy);
    }
    if hover_set_changed {
        projection.hovered_meshes = if selection.hover_color_change_enabled {
            hovered_target
                .anchor
                .as_ref()
                .map(|target| {
                    selected_meshes(std::slice::from_ref(target), &scene_index, &mesh_hierarchy)
                })
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
    }

    let desired = if state.enabled && state.visible {
        projection.selected_meshes.clone()
    } else {
        HashSet::new()
    };
    diagnostics
        .unsupported_entities
        .retain(|entity| desired.contains(entity));
    diagnostics
        .missing_material_entities
        .retain(|entity| desired.contains(entity));

    let mut deferred_clip_ids = HashSet::new();
    let stale_entities = projection
        .active_entities
        .difference(&desired)
        .copied()
        .collect::<Vec<_>>();
    for entity in stale_entities {
        if let Ok((_, _, clip, Some(underlying), ..)) = renderables.get(entity)
            && let Some(clip) = clip
        {
            deferred_clip_ids.insert(clip.0.id());
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<SectionClipMaterial>>()
                .insert(MeshMaterial3d(underlying.0.clone()))
                .remove::<SectionClipUnderlyingMaterial>();
        }
        projection.active_entities.remove(&entity);
    }

    let uniform = SectionClipUniform {
        world_to_box: state.transform.to_matrix().inverse(),
        enabled: 1,
        _padding: Vec3::ZERO,
    };
    let selection_handle = selection_material.as_ref().map(|material| &material.0);
    let hover_handle = hover_material.as_ref().map(|material| &material.0);
    let uniform_handle = uniform_material.as_ref().map(|material| &material.0);
    let mut next_active = HashSet::new();
    let mut used_base_ids = HashSet::new();

    for entity in &desired {
        let Ok((_, standard, clip, underlying, original, selection_base, selection_override)) =
            renderables.get(*entity)
        else {
            continue;
        };
        let Some(current_route) = underlying
            .map(|material| material.0.clone())
            .or_else(|| standard.map(|material| material.0.clone()))
        else {
            if diagnostics.unsupported_entities.insert(*entity) {
                bevy::log::warn!(
                    "[section-box] selected renderable {entity:?} has no StandardMaterial route; clipping skipped"
                );
            }
            continue;
        };

        let composed = compose_clipped_route(
            current_route.clone(),
            original,
            selection_base,
            selection_override.is_some(),
            selection.color_change_enabled,
            selection_handle,
            projection.selected_meshes.contains(entity),
            selection.hover_color_change_enabled,
            hover_handle,
            projection.hovered_meshes.contains(entity),
            toggles.renderer.render_mode,
            uniform_handle,
        );
        let Some(clip_handle) = ensure_clip_material(
            &composed.route,
            &standard_materials,
            &mut clip_materials,
            &mut projection.material_cache,
            uniform,
        ) else {
            if clip.is_some() {
                next_active.insert(*entity);
            } else if diagnostics.missing_material_entities.insert(*entity) {
                bevy::log::warn!(
                    "[section-box] selected renderable {entity:?} references a missing StandardMaterial; clipping skipped"
                );
            }
            continue;
        };

        used_base_ids.insert(composed.route.id());
        commands
            .entity(*entity)
            .insert((
                MeshMaterial3d(clip_handle),
                SectionClipUnderlyingMaterial(composed.route.clone()),
            ))
            .remove::<MeshMaterial3d<StandardMaterial>>();
        apply_composed_route(
            &mut commands,
            *entity,
            &composed,
            original,
            selection_base,
            selection_override.is_some(),
        );
        next_active.insert(*entity);
    }

    projection.active_entities = next_active;
    projection.last_targets = Some(state.targets.clone());
    projection.last_hovered_anchor = hovered_target.anchor.clone();
    projection.last_hover_enabled = Some(selection.hover_color_change_enabled);
    projection.last_state_revision = Some(state.revision);
    projection.last_scene_revision = Some(scene_index.revision());
    projection.last_presentation = Some(presentation);

    let mut in_use_clip_ids = deferred_clip_ids;
    for entity in &projection.active_entities {
        if let Ok((_, _, Some(clip), ..)) = renderables.get(*entity) {
            in_use_clip_ids.insert(clip.0.id());
        }
    }
    prune_material_cache(
        &mut projection.material_cache,
        &mut clip_materials,
        &used_base_ids,
        &in_use_clip_ids,
    );
}
