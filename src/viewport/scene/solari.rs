//! Bevy Solari capability ownership and renderer-neutral publication.
//!
//! Solari is opt-in at compile time and still requires the active wgpu device
//! to expose every feature requested by Bevy's plugin group. The main world
//! only publishes the resulting capability bit; it never exposes wgpu details
//! through the viewport protocol.

#[path = "solari_projection.rs"]
mod projection;
#[path = "solari_proof.rs"]
mod proof;

use bevy::prelude::*;
use bevy::render::render_resource::WgpuFeatures;
use bevy::render::renderer::RenderDevice;
use bevy::render::{ExtractSchedule, MainWorld, RenderApp};

use crate::viewport::api::ViewerSettingsState;
use crate::viewport::api::ViewportBridgeSet;

#[cfg(all(test, feature = "solari"))]
pub(crate) use projection::SolariProjectionStats;
use projection::sync_solari_usd_projection;
pub(crate) use projection::{SolariProjectionDiagnostics, SolariProjectionState};
#[cfg(all(test, feature = "solari"))]
pub(crate) use proof::SolariProofMesh;
#[cfg(feature = "solari")]
use proof::spawn_solari_proof_scene;
use proof::{
    SolariProofActivation, activate_proof_mode, sync_solari_camera, sync_solari_proof_meshes,
};

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
            .init_resource::<SolariProjectionDiagnostics>()
            .init_resource::<SolariProjectionState>()
            .init_resource::<SolariProofActivation>()
            .add_systems(
                Update,
                (
                    sync_solari_usd_projection,
                    report_projection_diagnostics,
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

fn report_projection_diagnostics(diagnostics: Res<SolariProjectionDiagnostics>) {
    if !diagnostics.is_changed() || diagnostics.unsupported_meshes == 0 {
        return;
    }
    warn!(
        candidate_meshes = diagnostics.candidate_meshes,
        eligible_meshes = diagnostics.eligible_meshes,
        unsupported_meshes = diagnostics.unsupported_meshes,
        missing_materials = diagnostics.missing_materials,
        "[solari] USD projection contains meshes outside the supported Solari subset; Ray Traced remains unavailable"
    );
}

fn publish_capability(
    capability: Res<SolariCapability>,
    settings: Option<Res<ViewerSettingsState>>,
    mut commands: Commands,
) {
    let Some(settings) = settings else {
        return;
    };
    let supported = capability.supported();
    if settings.ray_traced_supported() != supported {
        let mut next = (*settings).clone();
        next.set_ray_traced_supported(supported);
        commands.insert_resource(next);
    }
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

#[cfg(test)]
#[path = "solari_projection_tests.rs"]
mod projection_tests;
