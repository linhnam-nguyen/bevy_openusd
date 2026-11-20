//! Desktop input adapters for the native viewport.

mod headless_gizmo;
pub(crate) mod navigation;

pub(crate) use headless_gizmo::sync_headless_gizmo_input;
pub(crate) use navigation::{
    HostUiInputCapture, ViewportNavigationInput, apply_local_navigation_input,
    apply_remote_navigation_input, reset_navigation_frame, ui_wants_keyboard_input,
    ui_wants_pointer_input,
};
