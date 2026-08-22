//! Protocol-owned viewer settings state.
//!
//! This resource records authoritative settings vocabulary and capability
//! state without applying renderer integrations. Later B milestones can use
//! the same state as the handoff point for Bevy, Glacial, or vendor adapters.

use bevy::prelude::*;
use viewport_protocol::{
    SamplingPreference, SelectionPresentationSettings, SelectionReadModel,
    ViewerEnvironmentSettings, ViewerSettingsReadModel,
};

#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ViewerSettingsState(pub(crate) ViewerSettingsReadModel);

impl ViewerSettingsState {
    pub(crate) fn set_environment(&mut self, settings: ViewerEnvironmentSettings) {
        self.0.environment = settings;
    }

    pub(crate) fn set_sampling(&mut self, preference: SamplingPreference) {
        self.0.sampling.preference = preference;
        // Provider negotiation is deliberately not implemented in B1. The
        // authoritative fallback remains explicit rather than pretending a
        // vendor integration has been applied.
        self.0.sampling.provider = Default::default();
    }

    pub(crate) fn set_selection(&mut self, settings: SelectionPresentationSettings) {
        self.0.selection = settings;
    }

    pub(crate) fn set_section_box(&mut self, enabled: bool, selection: &SelectionReadModel) {
        self.0.section_box.enabled = enabled;
        self.0.section_box.targets = if enabled {
            selection.targets.clone()
        } else {
            Vec::new()
        };
    }

    pub(crate) fn sync_section_box_selection(&mut self, selection: &SelectionReadModel) -> bool {
        if !self.0.section_box.enabled {
            return false;
        }
        if self.0.section_box.targets == selection.targets {
            return false;
        }
        self.0.section_box.targets = selection.targets.clone();
        true
    }
}
