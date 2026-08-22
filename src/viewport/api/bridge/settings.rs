//! Protocol-owned supplementary viewer settings state.
//!
//! Renderer configuration and ground-grid origin remain owned by the existing
//! presentation resources. This resource stores only settings introduced by
//! Viewer Settings that do not have another authoritative owner yet.

use bevy::prelude::*;
use viewport_protocol::ViewerSettingsReadModel;

#[derive(Resource, Debug, Clone, Default)]
pub(super) struct ViewerSettingsState(pub(super) ViewerSettingsReadModel);
