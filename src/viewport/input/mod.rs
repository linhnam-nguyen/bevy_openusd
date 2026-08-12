//! Desktop input adapters for the native viewport.

pub(crate) mod keyboard;
pub(crate) mod navigation;

pub(crate) use navigation::{
    ViewportNavigationInput, apply_local_navigation_input, apply_remote_navigation_input,
    reset_navigation_frame,
};
