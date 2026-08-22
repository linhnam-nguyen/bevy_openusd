use viewport_protocol::ViewerEnvironmentSettings;

use super::super::ViewerSettingsState;
use super::super::helpers::emit_viewer_settings_changed;
use crate::viewport::api::ViewportEventOutbox;

pub(super) fn set_environment(
    request_id: String,
    settings: ViewerEnvironmentSettings,
    outbox: &mut ViewportEventOutbox,
    viewer_settings: &mut ViewerSettingsState,
) {
    viewer_settings.set_environment(settings);
    emit_viewer_settings_changed(outbox, request_id, &viewer_settings.0);
}
