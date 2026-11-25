//! Re-export of generic viewport settings state at the API bridge boundary.

pub(crate) use crate::viewport::presentation::ViewerSettingsState;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/api/bridge/settings_test_support.rs"]
mod test_support;
