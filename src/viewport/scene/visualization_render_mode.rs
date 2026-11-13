//! Renderer-owned mode projection for normal viewport presentation.
//!
//! Uniform Color temporarily rebinds projected USD meshes to one shared
//! material. Shaded restores each mesh's authored material handle, while
//! Wireframe continues to use Bevy's native wireframe configuration.

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::RenderMode;

use super::super::projection_work::{DirectProjectionWork, ProjectionBatch, ProjectionWork};
use super::super::selection_color::SelectionBaseMaterial;
use super::DisplayToggles;

const UNIFORM_COLOR: Color = Color::srgb(0.72, 0.72, 0.72);

#[derive(Resource, Debug, Clone)]
pub(in crate::viewport) struct UniformRenderMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct OriginalRenderMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

#[derive(Resource, Debug, Clone, Copy)]
pub(super) struct RenderModeProjectionState {
    last_mode: RenderMode,
}

impl Default for RenderModeProjectionState {
    fn default() -> Self {
        Self {
            last_mode: RenderMode::Shaded,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/visualization_render_mode_test_support.rs"]
mod test_support;
#[cfg(test)]
pub(super) use test_support::RenderModeProjectionStats;

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

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_render_mode(
    toggles: Res<DisplayToggles>,
    uniform: Option<Res<UniformRenderMaterial>>,
    projection: ResMut<RenderModeProjectionState>,
    commands: Commands,
    prims: ParamSet<(
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<UsdPrimRef>,
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            (
                With<UsdPrimRef>,
                Or<(Added<UsdPrimRef>, Changed<MeshMaterial3d<StandardMaterial>>)>,
            ),
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                &OriginalRenderMaterial,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<OriginalRenderMaterial>,
        >,
    )>,
) {
    apply_render_mode_core(
        toggles,
        uniform,
        projection,
        commands,
        prims,
        &mut DirectProjectionWork,
    );
}

#[allow(clippy::too_many_arguments)]
fn apply_render_mode_core(
    toggles: Res<DisplayToggles>,
    uniform: Option<Res<UniformRenderMaterial>>,
    mut projection: ResMut<RenderModeProjectionState>,
    mut commands: Commands,
    mut prims: ParamSet<(
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<UsdPrimRef>,
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            (
                With<UsdPrimRef>,
                Or<(Added<UsdPrimRef>, Changed<MeshMaterial3d<StandardMaterial>>)>,
            ),
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                &OriginalRenderMaterial,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<OriginalRenderMaterial>,
        >,
    )>,
    work: &mut impl ProjectionWork,
) {
    let desired = toggles.renderer.render_mode;
    let mode_changed = projection.last_mode != desired;

    if mode_changed {
        if desired == RenderMode::UniformColor {
            let Some(uniform) = uniform.as_ref() else {
                return;
            };
            projection.last_mode = desired;
            let mut prims = prims.p0();
            work.execute(
                ProjectionBatch::MaterialTransition,
                &mut prims,
                |(entity, mut material, original, selection_base)| {
                    if material.0 == uniform.0 {
                        return;
                    }

                    if let Some(mut selection_base) = selection_base {
                        if original.is_none() {
                            commands
                                .entity(entity)
                                .insert(OriginalRenderMaterial(selection_base.0.clone()));
                        }
                        selection_base.0 = uniform.0.clone();
                        return;
                    }

                    if let Some(mut original) = original {
                        original.0 = material.0.clone();
                    } else {
                        commands
                            .entity(entity)
                            .insert(OriginalRenderMaterial(material.0.clone()));
                    }
                    material.0 = uniform.0.clone();
                },
            );
        } else {
            projection.last_mode = desired;
            let mut prims = prims.p2();
            // Ray Traced is rejected by the B2 protocol validator and remains
            // a B3 capability. If an internal caller places it in this state,
            // restore the authored material without claiming ray tracing.
            work.execute(
                ProjectionBatch::MaterialRestore,
                &mut prims,
                |(entity, mut material, original, selection_base)| {
                    if let Some(mut selection_base) = selection_base {
                        selection_base.0 = original.0.clone();
                    } else {
                        material.0 = original.0.clone();
                    }
                    commands.entity(entity).remove::<OriginalRenderMaterial>();
                },
            );
        }
        return;
    }

    if desired == RenderMode::UniformColor {
        let Some(uniform) = uniform.as_ref() else {
            return;
        };
        let mut prims = prims.p1();
        work.execute(
            ProjectionBatch::MaterialIncremental,
            &mut prims,
            |(entity, mut material, original, selection_base)| {
                if material.0 == uniform.0 {
                    if let Some(mut selection_base) = selection_base {
                        selection_base.0 = uniform.0.clone();
                    }
                    return;
                }

                if let Some(mut selection_base) = selection_base {
                    if original.is_none() {
                        commands
                            .entity(entity)
                            .insert(OriginalRenderMaterial(selection_base.0.clone()));
                    }
                    selection_base.0 = uniform.0.clone();
                    return;
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
            },
        );
    }
}

pub(super) fn apply_wireframe_toggle(
    toggles: Res<DisplayToggles>,
    mut config: ResMut<bevy::pbr::wireframe::WireframeConfig>,
) {
    let desired = toggles.renderer.render_mode == RenderMode::Wireframe;
    if config.global != desired {
        config.global = desired;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/visualization_render_mode_tests.rs"]
mod tests;
