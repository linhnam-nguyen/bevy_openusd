//! Renderer-only six-plane clipping through an Extended StandardMaterial.

use std::collections::{HashMap, HashSet};

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::section_box::SectionBoxState;
use super::selection_outline::collect_mesh_descendants;
use crate::viewport::api::SceneAnchorIndex;

const SHADER_ASSET_PATH: &str = "shaders/section_box_clipping.wgsl";

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
        SHADER_ASSET_PATH.into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }
}

#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct SectionClipOriginalMaterial(Handle<StandardMaterial>);

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct SectionClipProjectionState {
    clipped_entities: HashSet<Entity>,
    material_cache: HashMap<AssetId<StandardMaterial>, Handle<SectionClipMaterial>>,
}

/// Applies one aggregate box-space test to every selected renderable while
/// preserving each mesh's original StandardMaterial for restoration.
#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_section_box_clipping(
    state: Res<SectionBoxState>,
    scene_index: Res<SceneAnchorIndex>,
    mut projection: ResMut<SectionClipProjectionState>,
    mut commands: Commands,
    standard_materials: Res<Assets<StandardMaterial>>,
    mut clip_materials: ResMut<Assets<SectionClipMaterial>>,
    mesh_hierarchy: Query<(Option<&Mesh3d>, Option<&Children>)>,
    standard_meshes: Query<(Entity, &MeshMaterial3d<StandardMaterial>), With<Mesh3d>>,
    clipped_meshes: Query<
        (Entity, &SectionClipOriginalMaterial),
        (With<Mesh3d>, With<SectionClipOriginalMaterial>),
    >,
) {
    let desired = if state.enabled && state.visible {
        selected_meshes(&state.targets, &scene_index, &mesh_hierarchy)
    } else {
        HashSet::new()
    };

    for (entity, original) in &clipped_meshes {
        if !desired.contains(&entity) {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<SectionClipMaterial>>()
                .insert(MeshMaterial3d(original.0.clone()))
                .remove::<SectionClipOriginalMaterial>();
            projection.clipped_entities.remove(&entity);
        }
    }

    if desired.is_empty() {
        projection.material_cache.clear();
        return;
    }

    let uniform = SectionClipUniform {
        world_to_box: state.transform.to_matrix().inverse(),
        enabled: 1,
        _padding: Vec3::ZERO,
    };
    for clip_handle in projection.material_cache.values() {
        if let Some(mut clip_material) = clip_materials.get_mut(clip_handle) {
            clip_material.extension.clip = uniform;
        }
    }
    for (entity, material) in &standard_meshes {
        if !desired.contains(&entity) {
            continue;
        }
        let Some(base) = standard_materials.get(&material.0).cloned() else {
            continue;
        };
        let clip_handle = projection
            .material_cache
            .entry(material.0.id())
            .or_insert_with(|| {
                clip_materials.add(ExtendedMaterial {
                    base,
                    extension: SectionClipExtension { clip: uniform },
                })
            })
            .clone();
        if let Some(mut clip_material) = clip_materials.get_mut(&clip_handle) {
            clip_material.extension.clip = uniform;
        }
        commands
            .entity(entity)
            .insert((
                MeshMaterial3d(clip_handle),
                SectionClipOriginalMaterial(material.0.clone()),
            ))
            .remove::<MeshMaterial3d<StandardMaterial>>();
        projection.clipped_entities.insert(entity);
    }
}

fn selected_meshes(
    targets: &[viewport_protocol::SceneAnchor],
    scene_index: &SceneAnchorIndex,
    mesh_hierarchy: &Query<(Option<&Mesh3d>, Option<&Children>)>,
) -> HashSet<Entity> {
    let mut selected = HashSet::new();
    for target in targets {
        let Some(root) = scene_index.resolve(target) else {
            continue;
        };
        collect_mesh_descendants(root, mesh_hierarchy, &mut selected);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_uniform_uses_one_shared_box_space_transform() {
        let transform =
            Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)).with_scale(Vec3::splat(6.0));
        let uniform = SectionClipUniform {
            world_to_box: transform.to_matrix().inverse(),
            enabled: 1,
            _padding: Vec3::ZERO,
        };

        assert_eq!(uniform.enabled, 1);
        assert!(
            uniform
                .world_to_box
                .transform_point3(transform.translation)
                .abs_diff_eq(Vec3::ZERO, 0.0001)
        );
    }

    #[test]
    fn extension_keeps_the_standard_material_as_the_base_route() {
        let base = StandardMaterial {
            base_color: Color::srgb(0.2, 0.4, 0.8),
            perceptual_roughness: 0.35,
            ..default()
        };
        let extended = ExtendedMaterial {
            base: base.clone(),
            extension: SectionClipExtension::default(),
        };

        assert_eq!(extended.base.base_color, base.base_color);
        assert_eq!(
            extended.base.perceptual_roughness,
            base.perceptual_roughness
        );
    }
}
