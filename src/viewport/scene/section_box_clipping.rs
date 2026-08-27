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

use super::selection_color::{
    HoverColorMaterial, SelectionBaseMaterial, SelectionColorMaterial, SelectionColorOverride,
};
use super::selection_projection::SelectedRenderableProjection;
use super::visualization::{OriginalRenderMaterial, UniformRenderMaterial};

#[path = "section_box_clipping_support.rs"]
mod support;

#[cfg(test)]
#[path = "section_box_clipping_tests.rs"]
mod tests;

#[path = "section_box_clipping_sync.rs"]
mod sync;

pub(in crate::viewport) use sync::sync_section_box_clipping;

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
    last_projection_generation: Option<u64>,
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
