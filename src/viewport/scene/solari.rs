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

use crate::viewport::api::ViewerSettingsState;
use crate::viewport::api::ViewportBridgeSet;

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
        app.init_resource::<SolariCapability>().add_systems(
            Update,
            (refresh_scene_eligibility, publish_capability)
                .chain()
                .before(ViewportBridgeSet::ApplyCommands),
        );

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(ExtractSchedule, probe_render_device);
        }
    }
}

fn refresh_scene_eligibility(
    meshes: Option<Res<Assets<Mesh>>>,
    query: Query<(&Mesh3d, Option<&MeshMaterial3d<StandardMaterial>>), With<UsdPrimRef>>,
    mut capability: ResMut<SolariCapability>,
) {
    let Some(meshes) = meshes else {
        capability.scene_eligible = false;
        return;
    };

    let mut mesh_count = 0;
    let mut eligible = true;
    for (mesh_handle, material) in &query {
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
