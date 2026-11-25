//! Renderer-only six-plane clipping through a compositional Extended StandardMaterial.

use std::collections::{HashMap, HashSet};

use bevy::asset::{AssetEvent, Handle, load_internal_asset, uuid_handle};
use bevy::ecs::system::SystemParam;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::{Shader, ShaderRef};
use viewport_protocol::{ColorRgb8, RenderMode, SceneAnchor};

use super::super::selection::{
    HoverColorMaterial, SelectedRenderableProjection, SelectionBaseMaterial,
    SelectionColorMaterial, SelectionColorOverride,
};
use crate::viewport::rendering::scene::visualization::{OriginalRenderMaterial, UniformRenderMaterial};

mod support;

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/section_box/clipping_tests.rs"]
mod tests;

mod sync;

pub(crate) use sync::sync_section_box_clipping;

pub(crate) const SECTION_CLIP_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("d4f7b9a1-4c2e-4f8a-9b12-5d8e3a1c7f01");
pub(crate) const SECTION_CLIP_PREPASS_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("d4f7b9a1-4c2e-4f8a-9b12-5d8e3a1c7f02");

pub(crate) fn register_embedded_shaders(app: &mut App) {
    if !app.world().contains_resource::<Assets<Shader>>() {
        app.init_asset::<Shader>();
    }
    load_internal_asset!(
        app,
        SECTION_CLIP_SHADER_HANDLE,
        "../../../../../assets/shaders/section_box_clipping.wesl",
        Shader::from_wesl
    );
    load_internal_asset!(
        app,
        SECTION_CLIP_PREPASS_SHADER_HANDLE,
        "../../../../../assets/shaders/section_box_clipping_prepass.wesl",
        Shader::from_wesl
    );
}

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
struct SectionClipUniform {
    world_to_box: Mat4,
    enabled: u32,
    _padding: Vec3,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(crate) struct SectionClipExtension {
    #[uniform(100)]
    clip: SectionClipUniform,
}

impl SectionClipExtension {
    pub(crate) fn keeps_world_point(&self, point: Vec3) -> bool {
        if self.clip.enabled == 0 {
            return true;
        }
        let point = self.clip.world_to_box.transform_point3(point);
        point.cmpge(Vec3::splat(-0.5)).all() && point.cmple(Vec3::splat(0.5)).all()
    }
}

pub(crate) type SectionClipMaterial =
    ExtendedMaterial<StandardMaterial, SectionClipExtension>;

impl MaterialExtension for SectionClipExtension {
    fn fragment_shader() -> ShaderRef {
        SECTION_CLIP_SHADER_HANDLE.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        SECTION_CLIP_PREPASS_SHADER_HANDLE.into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        SECTION_CLIP_SHADER_HANDLE.into()
    }
}

/// The StandardMaterial route that remains visible below the clipping wrapper.
/// It is the composition boundary for the frozen B2/B5 presentation systems.
#[derive(Component, Debug, Clone)]
pub(crate) struct SectionClipUnderlyingMaterial(
    pub(crate) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Default)]
pub(crate) struct SectionClipProjectionState {
    active_entities: HashSet<Entity>,
    pick_revision: u64,
    selected_meshes: HashSet<Entity>,
    hovered_meshes: HashSet<Entity>,
    material_cache: HashMap<AssetId<StandardMaterial>, Handle<SectionClipMaterial>>,
    last_targets: Option<Vec<SceneAnchor>>,
    last_hovered_anchor: Option<SceneAnchor>,
    last_hover_enabled: Option<bool>,
    last_state_revision: Option<u64>,
    last_scene_revision: Option<u64>,
    last_projection_generation: Option<u64>,
    last_presentation: Option<SectionClipPresentationKey>,
}

impl SectionClipProjectionState {
    pub(crate) fn pick_revision(&self) -> u64 {
        self.pick_revision
    }
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
pub(crate) struct SectionClipDiagnostics {
    pub(crate) unsupported_entities: HashSet<Entity>,
    pub(crate) missing_material_entities: HashSet<Entity>,
}

#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub(crate) struct SectionClipSystemParam<'w, 's> {
    selection_material: Option<Res<'w, SelectionColorMaterial>>,
    hover_material: Option<Res<'w, HoverColorMaterial>>,
    uniform_material: Option<Res<'w, UniformRenderMaterial>>,
    projection: ResMut<'w, SectionClipProjectionState>,
    selected_projection: Option<Res<'w, SelectedRenderableProjection>>,
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
