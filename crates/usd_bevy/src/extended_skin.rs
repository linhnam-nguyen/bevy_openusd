//! Production extended skinning material for bindings that require more than
//! Bevy's native four-influence vertex group.

use bevy::asset::{Handle, load_internal_asset, uuid_handle};
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MaterialPlugin, StandardMaterial,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};

use crate::mesh::{
    ATTRIBUTE_EXTENDED_JOINT_INDEX_1, ATTRIBUTE_EXTENDED_JOINT_INDEX_2,
    ATTRIBUTE_EXTENDED_JOINT_INDEX_3, ATTRIBUTE_EXTENDED_JOINT_WEIGHT_1,
    ATTRIBUTE_EXTENDED_JOINT_WEIGHT_2, ATTRIBUTE_EXTENDED_JOINT_WEIGHT_3,
};

pub const EXTENDED_SKIN_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("c6b0e8e2-1e9c-4ae4-93b3-9b7f3f0c1616");

/// Marker for a mesh selected by the data-driven fidelity classifier.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct ExtendedSkinMesh;

pub type ExtendedSkinMaterial = ExtendedMaterial<StandardMaterial, ExtendedSkinExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct ExtendedSkinExtension {}

impl MaterialExtension for ExtendedSkinExtension {
    fn vertex_shader() -> ShaderRef {
        EXTENDED_SKIN_SHADER_HANDLE.into()
    }

    // The base StandardMaterial shaders retain equivalent prepass and shadow
    // passes; only the forward vertex path is replaced for the four groups.
    fn enable_prepass() -> bool {
        true
    }

    fn enable_shadows() -> bool {
        true
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let mut attributes = Vec::new();
        if layout.0.contains(Mesh::ATTRIBUTE_POSITION) {
            attributes.push(Mesh::ATTRIBUTE_POSITION.at_shader_location(0));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_NORMAL) {
            attributes.push(Mesh::ATTRIBUTE_NORMAL.at_shader_location(1));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_UV_0) {
            attributes.push(Mesh::ATTRIBUTE_UV_0.at_shader_location(2));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_UV_1) {
            attributes.push(Mesh::ATTRIBUTE_UV_1.at_shader_location(3));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_TANGENT) {
            attributes.push(Mesh::ATTRIBUTE_TANGENT.at_shader_location(4));
        }
        if layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(5));
        }
        attributes.extend([
            Mesh::ATTRIBUTE_JOINT_INDEX.at_shader_location(6),
            Mesh::ATTRIBUTE_JOINT_WEIGHT.at_shader_location(7),
            ATTRIBUTE_EXTENDED_JOINT_INDEX_1.at_shader_location(8),
            ATTRIBUTE_EXTENDED_JOINT_WEIGHT_1.at_shader_location(9),
            ATTRIBUTE_EXTENDED_JOINT_INDEX_2.at_shader_location(10),
            ATTRIBUTE_EXTENDED_JOINT_WEIGHT_2.at_shader_location(11),
            ATTRIBUTE_EXTENDED_JOINT_INDEX_3.at_shader_location(12),
            ATTRIBUTE_EXTENDED_JOINT_WEIGHT_3.at_shader_location(13),
        ]);
        descriptor.vertex.buffers = vec![layout.0.get_layout(&attributes)?];
        Ok(())
    }
}

pub struct ExtendedSkinPlugin;

impl Plugin for ExtendedSkinPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            EXTENDED_SKIN_SHADER_HANDLE,
            "extended_skin.wgsl",
            Shader::from_wgsl
        );
        app.add_plugins(MaterialPlugin::<ExtendedSkinMaterial>::default());
    }
}

pub fn set_extended_material(world: &mut World, entity: Entity, base: StandardMaterial) -> bool {
    let Some(handle) = world
        .get_resource_mut::<Assets<ExtendedSkinMaterial>>()
        .map(|mut assets| {
            assets.add(ExtendedMaterial {
                base,
                extension: ExtendedSkinExtension {},
            })
        })
    else {
        return false;
    };
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return false;
    };
    entity_mut.insert(MeshMaterial3d(handle));
    entity_mut.remove::<MeshMaterial3d<StandardMaterial>>();
    true
}
