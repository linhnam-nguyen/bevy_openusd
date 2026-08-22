//! Renderer-owned mode projection for normal viewport presentation.
//!
//! Uniform Color temporarily rebinds projected USD meshes to one shared
//! material. Shaded restores each mesh's authored material handle, while
//! Wireframe continues to use Bevy's native wireframe configuration.

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::RenderMode;

use super::DisplayToggles;

const UNIFORM_COLOR: Color = Color::srgb(0.72, 0.72, 0.72);

#[derive(Resource, Debug, Clone)]
pub(super) struct UniformRenderMaterial(Handle<StandardMaterial>);

#[derive(Component, Debug, Clone)]
pub(super) struct OriginalRenderMaterial(Handle<StandardMaterial>);

pub(super) fn init_uniform_render_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Option<Res<UniformRenderMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    commands.insert_resource(UniformRenderMaterial(materials.add(StandardMaterial {
        base_color: UNIFORM_COLOR,
        perceptual_roughness: 1.0,
        ..default()
    })));
}

pub(super) fn apply_render_mode(
    toggles: Res<DisplayToggles>,
    uniform: Option<Res<UniformRenderMaterial>>,
    mut commands: Commands,
    mut prims: Query<
        (
            Entity,
            &mut MeshMaterial3d<StandardMaterial>,
            Option<&mut OriginalRenderMaterial>,
        ),
        With<UsdPrimRef>,
    >,
) {
    match toggles.renderer.render_mode {
        RenderMode::UniformColor => {
            let Some(uniform) = uniform else {
                return;
            };

            for (entity, mut material, original) in &mut prims {
                if material.0 == uniform.0 {
                    continue;
                }

                if let Some(mut original) = original {
                    // Preserve a material route update that arrived while the
                    // uniform mode was active.
                    original.0 = material.0.clone();
                } else {
                    commands
                        .entity(entity)
                        .insert(OriginalRenderMaterial(material.0.clone()));
                }
                material.0 = uniform.0.clone();
            }
        }
        RenderMode::Shaded | RenderMode::Wireframe | RenderMode::RayTraced => {
            // Ray Traced is rejected by the B2 protocol validator and remains
            // a B3 capability. If an internal caller places it in this state,
            // restore the authored material without claiming ray tracing.
            for (entity, mut material, original) in &mut prims {
                if let Some(original) = original {
                    material.0 = original.0.clone();
                    commands.entity(entity).remove::<OriginalRenderMaterial>();
                }
            }
        }
    }
}

pub(super) fn apply_wireframe_toggle(
    toggles: Res<DisplayToggles>,
    mut config: ResMut<bevy::pbr::wireframe::WireframeConfig>,
) {
    config.global = toggles.renderer.render_mode == RenderMode::Wireframe;
}

#[cfg(test)]
#[path = "visualization_render_mode_tests.rs"]
mod tests;
