//! Bevy Solari capability ownership and renderer-neutral publication.
//!
//! Solari is opt-in at compile time and still requires the active wgpu device
//! to expose every feature requested by Bevy's plugin group. The main world
//! only publishes the resulting capability bit; it never exposes wgpu details
//! through the viewport protocol.

use bevy::mesh::{Indices, Mesh, Mesh3d, PrimitiveTopology};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::WgpuFeatures;
use bevy::render::renderer::RenderDevice;
use bevy::render::{ExtractSchedule, MainWorld, RenderApp};
use usd_bevy::UsdPrimRef;
use viewport_protocol::RenderMode;

use crate::viewport::api::ViewerSettingsState;
use crate::viewport::api::ViewportBridgeSet;
use crate::viewport::scene::visualization::DisplayToggles;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariProofMesh;

#[derive(Resource, Debug, Default, Clone, Copy)]
struct SolariProofActivation {
    activated: bool,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariCapability {
    pub(crate) compiled: bool,
    pub(crate) device_supported: bool,
    pub(crate) scene_eligible: bool,
}

impl Default for SolariCapability {
    fn default() -> Self {
        Self {
            compiled: cfg!(feature = "solari"),
            device_supported: false,
            scene_eligible: false,
        }
    }
}

impl SolariCapability {
    pub(crate) fn supported(self) -> bool {
        self.compiled && self.device_supported && self.scene_eligible
    }
}

pub(crate) struct SolariCapabilityPlugin;

impl Plugin for SolariCapabilityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SolariCapability>()
            .init_resource::<SolariProofActivation>()
            .add_systems(
                Update,
                (
                    refresh_scene_eligibility,
                    publish_capability,
                    activate_proof_mode,
                    sync_solari_camera,
                    sync_solari_proof_meshes,
                )
                    .chain()
                    .before(ViewportBridgeSet::ApplyCommands),
            );

        #[cfg(feature = "solari")]
        app.add_systems(Startup, spawn_solari_proof_scene);

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(ExtractSchedule, probe_render_device);
        }
    }
}

fn refresh_scene_eligibility(
    meshes: Option<Res<Assets<Mesh>>>,
    mut queries: ParamSet<(
        Query<(&Mesh3d, Option<&MeshMaterial3d<StandardMaterial>>), With<UsdPrimRef>>,
        Query<(&Mesh3d, Option<&MeshMaterial3d<StandardMaterial>>), With<SolariProofMesh>>,
    )>,
    mut capability: ResMut<SolariCapability>,
) {
    let Some(meshes) = meshes else {
        capability.scene_eligible = false;
        return;
    };

    let mut mesh_count = 0;
    let mut eligible = true;
    for (mesh_handle, material) in &mut queries.p0() {
        mesh_count += 1;
        eligible &= material.is_some()
            && meshes
                .get(&mesh_handle.0)
                .is_some_and(mesh_is_solari_compatible);
    }
    for (mesh_handle, material) in &mut queries.p1() {
        mesh_count += 1;
        eligible &= material.is_some()
            && meshes
                .get(&mesh_handle.0)
                .is_some_and(mesh_is_solari_compatible);
    }
    capability.scene_eligible = mesh_count > 0 && eligible;
}

fn mesh_is_solari_compatible(mesh: &Mesh) -> bool {
    mesh.primitive_topology() == PrimitiveTopology::TriangleList
        && matches!(mesh.indices(), Some(Indices::U32(_)))
        && mesh.contains_attribute(Mesh::ATTRIBUTE_POSITION)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_NORMAL)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_UV_0)
        && mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT)
}

fn publish_capability(
    capability: Res<SolariCapability>,
    settings: Option<ResMut<ViewerSettingsState>>,
) {
    let Some(mut settings) = settings else {
        return;
    };
    settings.set_ray_traced_supported(capability.supported());
}

fn activate_proof_mode(
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
fn sync_solari_camera(
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
fn sync_solari_camera() {}

#[cfg(feature = "solari")]
fn sync_solari_proof_meshes(
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
fn sync_solari_proof_meshes() {}

#[cfg(feature = "solari")]
fn spawn_solari_proof_scene(
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

fn proof_scene_requested() -> bool {
    std::env::var("BEVY_OPENUSD_SOLARI_PROOF")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

fn probe_render_device(mut main_world: ResMut<MainWorld>, render_device: Res<RenderDevice>) {
    let device_supported =
        cfg!(feature = "solari") && render_device.features().contains(required_wgpu_features());
    let Some(mut capability) = main_world.get_resource_mut::<SolariCapability>() else {
        return;
    };
    capability.device_supported = device_supported;
}

fn required_wgpu_features() -> WgpuFeatures {
    #[cfg(feature = "solari")]
    {
        bevy::solari::prelude::SolariPlugins::required_wgpu_features()
    }
    #[cfg(not(feature = "solari"))]
    {
        WgpuFeatures::empty()
    }
}

#[cfg(test)]
#[path = "solari_tests.rs"]
mod tests;
