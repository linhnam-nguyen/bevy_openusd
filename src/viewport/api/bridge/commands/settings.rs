use viewport_protocol::{
    SamplingPreference, SelectionPresentationSettings, ViewerEnvironmentSettings,
};

use super::super::helpers::emit_viewer_settings_changed;
use super::SelectionState;
use crate::viewport::api::ViewportEventOutbox;

pub(super) fn set_environment(
    request_id: String,
    settings: ViewerEnvironmentSettings,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
) {
    let mut viewer_settings = selection_state.p2();
    viewer_settings.set_environment(settings);
    emit_viewer_settings_changed(outbox, request_id, &viewer_settings.0);
}

pub(super) fn set_sampling(
    request_id: String,
    preference: SamplingPreference,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
) {
    let mut viewer_settings = selection_state.p2();
    viewer_settings.set_sampling(preference);
    emit_viewer_settings_changed(outbox, request_id, &viewer_settings.0);
}

pub(super) fn set_selection(
    request_id: String,
    settings: SelectionPresentationSettings,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
) {
    let mut viewer_settings = selection_state.p2();
    viewer_settings.set_selection(settings);
    emit_viewer_settings_changed(outbox, request_id, &viewer_settings.0);
}

pub(super) fn set_section_box(
    request_id: String,
    enabled: bool,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
) {
    let selection = selection_state.p1().0.clone();
    let mut viewer_settings = selection_state.p2();
    viewer_settings.set_section_box(enabled, &selection);
    emit_viewer_settings_changed(outbox, request_id, &viewer_settings.0);
}
