//! Controlled Solari proof-scene and camera lifecycle.

#[cfg(feature = "solari")]
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use super::SolariCapability;
use crate::viewport::scene::visualization::DisplayToggles;
use viewport_protocol::RenderMode;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariProofMesh;

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct SolariProofActivation {
    activated: bool,
}

pub(super) fn activate_proof_mode(
    capability: Res<SolariCapability>,
    mut activation: ResMut<SolariProofActivation>,
    toggles: Option<ResMut<DisplayToggles>>,
) {
    if !proof_scene_requested() || activation.activated || !capability.supported() {
        return;
    }
    let Some(mut toggles) = toggles else {
        return;
    };
    if toggles.renderer.render_mode == RenderMode::Shaded {
        toggles.renderer.render_mode = RenderMode::RayTraced;
        activation.activated = true;
    }
}

#[cfg(feature = "solari")]
pub(super) fn sync_solari_camera(
    capability: Res<SolariCapability>,
    toggles: Res<DisplayToggles>,
    mut commands: Commands,
    cameras: Query<(Entity, Option<&bevy::solari::prelude::SolariLighting>), With<Camera3d>>,
) {
    let enabled = capability.supported() && toggles.renderer.render_mode == RenderMode::RayTraced;
    for (entity, lighting) in &cameras {
        match (enabled, lighting.is_some()) {
            (true, false) => {
                commands
                    .entity(entity)
                    .insert(bevy::solari::prelude::SolariLighting::default());
            }
            (false, true) => {
                commands
                    .entity(entity)
                    .remove::<bevy::solari::prelude::SolariLighting>()
                    .remove::<bevy::core_pipeline::prepass::DeferredPrepass>()
                    .remove::<bevy::core_pipeline::prepass::DepthPrepass>()
                    .remove::<bevy::core_pipeline::prepass::MotionVectorPrepass>()
                    .remove::<bevy::core_pipeline::prepass::DeferredPrepassDoubleBuffer>()
                    .remove::<bevy::core_pipeline::prepass::DepthPrepassDoubleBuffer>();
            }
            _ => {}
        }
    }
}

#[cfg(not(feature = "solari"))]
pub(super) fn sync_solari_camera() {}

#[cfg(feature = "solari")]
pub(super) fn sync_solari_proof_meshes(
    capability: Res<SolariCapability>,
    toggles: Res<DisplayToggles>,
    mut commands: Commands,
    meshes: Query<
        (
            Entity,
            &Mesh3d,
            Option<&bevy::solari::prelude::RaytracingMesh3d>,
        ),
        (
            With<SolariProofMesh>,
            With<MeshMaterial3d<StandardMaterial>>,
        ),
    >,
) {
    let enabled = capability.supported() && toggles.renderer.render_mode == RenderMode::RayTraced;
    for (entity, mesh, raytracing_mesh) in &meshes {
        if enabled && raytracing_mesh.is_none() {
            commands
                .entity(entity)
                .insert(bevy::solari::prelude::RaytracingMesh3d(mesh.0.clone()));
        } else if !enabled && raytracing_mesh.is_some() {
            commands
                .entity(entity)
                .remove::<bevy::solari::prelude::RaytracingMesh3d>();
        }
    }
}

#[cfg(not(feature = "solari"))]
pub(super) fn sync_solari_proof_meshes() {}

#[cfg(feature = "solari")]
pub(super) fn spawn_solari_proof_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !proof_scene_requested() {
        return;
    }
    let mesh = meshes.add(
        Sphere::new(0.75)
            .mesh()
            .build()
            .with_generated_tangents()
            .expect("Solari proof sphere must provide generated tangents"),
    );
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.48, 0.92),
        perceptual_roughness: 0.32,
        metallic: 0.12,
        ..default()
    });
    commands.spawn((
        Name::new("SolariProofSphere"),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 0.75, 0.0),
        SolariProofMesh,
    ));
}

#[cfg(not(feature = "solari"))]
pub(super) fn spawn_solari_proof_scene() {}

pub(super) fn proof_scene_requested() -> bool {
    std::env::var("BEVY_OPENUSD_SOLARI_PROOF")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}
