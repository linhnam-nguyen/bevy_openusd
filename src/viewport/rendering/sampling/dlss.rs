//! Optional Bevy DLSS provider boundary.
//!
//! The package feature only enables the integration when a build explicitly
//! opts into it. Runtime support is still fail-closed and comes exclusively
//! from Bevy's `DlssSuperResolutionSupported` resource after the Vulkan
//! renderer has initialized.

use bevy::prelude::*;

/// Renderer capability exposed to the sampling coordinator.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DlssCapability {
    pub(crate) compiled: bool,
    pub(crate) runtime_supported: bool,
}

impl Default for DlssCapability {
    fn default() -> Self {
        Self::from_probe(cfg!(feature = "dlss"), false)
    }
}

impl DlssCapability {
    pub(crate) const fn from_probe(compiled: bool, runtime_supported: bool) -> Self {
        Self {
            compiled,
            runtime_supported,
        }
    }

    pub(crate) const fn supported(self) -> bool {
        self.compiled && self.runtime_supported
    }
}

/// Renderer-local request consumed by the DLSS camera adapter.
///
/// B4.4's bridge command is the authoritative writer; this adapter only
/// applies the selected request when the runtime capability is present.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DlssCameraActivation {
    pub(crate) enabled: bool,
}

/// Registers Bevy's required pre-render DLSS initialization when the optional
/// package feature is enabled. The project ID is public application metadata;
/// it is intentionally distinct from Bevy's example ID.
pub(crate) fn configure_dlss(app: &mut App) {
    #[cfg(feature = "dlss")]
    app.insert_resource(bevy::anti_alias::dlss::DlssProjectId(
        bevy_asset::uuid::uuid!("9b7f6d1a-2f54-4c6e-8be2-8a49f7e3d1c0"),
    ))
    .add_plugins(bevy::anti_alias::dlss::DlssInitPlugin);
    #[cfg(not(feature = "dlss"))]
    let _ = app;
}

/// Installs the runtime probe and the camera-side provider adapter.
pub(crate) struct DlssProviderPlugin;

impl Plugin for DlssProviderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DlssCapability>()
            .init_resource::<DlssCameraActivation>()
            .add_systems(
                Update,
                refresh_capability.before(super::publish_sampling_capabilities),
            )
            .add_systems(
                Update,
                apply_camera_activation
                    .after(crate::viewport::api::ViewportBridgeSet::ApplyCommands)
                    .before(crate::viewport::api::ViewportBridgeSet::ReduceEvents),
            );
    }
}

#[cfg(feature = "dlss")]
fn refresh_capability(
    mut capability: ResMut<DlssCapability>,
    supported: Option<Res<bevy::anti_alias::dlss::DlssSuperResolutionSupported>>,
) {
    let runtime_supported = supported.is_some();
    if capability.runtime_supported != runtime_supported {
        capability.runtime_supported = runtime_supported;
    }
}

#[cfg(not(feature = "dlss"))]
fn refresh_capability(mut capability: ResMut<DlssCapability>) {
    capability.runtime_supported = false;
}

#[cfg(feature = "dlss")]
fn apply_camera_activation(
    capability: Res<DlssCapability>,
    activation: Res<DlssCameraActivation>,
    mut commands: Commands,
    cameras: Query<
        (Entity, Option<&bevy::anti_alias::dlss::Dlss>),
        With<crate::viewport::camera::ArcballCamera>,
    >,
) {
    let enabled = activation.enabled && capability.supported();
    for (entity, dlss) in cameras {
        match (enabled, dlss.is_some()) {
            (true, false) => {
                commands
                    .entity(entity)
                    .insert(bevy::anti_alias::dlss::Dlss::default());
            }
            (false, true) => {
                commands
                    .entity(entity)
                    .remove::<bevy::anti_alias::dlss::Dlss>();
            }
            _ => {}
        }
    }
}

#[cfg(not(feature = "dlss"))]
fn apply_camera_activation() {}

#[cfg(test)]
#[path = "dlss_tests.rs"]
mod tests;
