//! Native Bevy viewport for OpenUSD scenes.
//!
//! This module is the boundary between the `usdview` binary and the native
//! viewport implementation. Feature modules use explicit viewport-local paths
//! rather than importing through the binary root.

mod app;

#[cfg(test)]
pub(crate) use app::{
    ProductionActivationWorld, apply_prepared_activation_for_test,
    observe_project_activation_for_test,
};

pub(crate) mod animation;
pub(crate) mod api;
pub(crate) mod bim;
pub(crate) mod camera;
pub(crate) mod diagnostics;
pub(crate) mod input;
pub(crate) mod physics;
pub(crate) mod rendering;
pub(crate) mod scene;
pub(crate) mod semantic;
pub(crate) mod session;
pub(crate) mod transport;
pub(crate) mod ui_frost;

pub(crate) use app::run;
