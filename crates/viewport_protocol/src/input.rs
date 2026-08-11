//! Renderer-neutral pointer, wheel, keyboard, and focus messages.

use serde::{Deserialize, Serialize};

/// Pointer buttons accompanying reliable input messages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerButtons {
    pub primary: bool,
    pub secondary: bool,
    pub auxiliary: bool,
}

/// Keyboard modifier state shared by browser and native adapters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Coalesced low-latency motion packet. Deltas are CSS pixels, not screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerMotion {
    pub sequence: u64,
    pub dx_css_pixels: f32,
    pub dy_css_pixels: f32,
    pub wheel_x: f32,
    pub wheel_y: f32,
    pub viewport_css_width: f32,
    pub viewport_css_height: f32,
    pub stream_generation: u64,
}

/// Reliable pointer button state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonState {
    pub sequence: u64,
    pub buttons: PointerButtons,
    pub modifiers: InputModifiers,
    pub stream_generation: u64,
}

/// Reliable keyboard transition using browser-independent code/key strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardInput {
    pub sequence: u64,
    pub code: String,
    pub key: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: InputModifiers,
    pub stream_generation: u64,
}

/// Focus lifecycle state used to clear stuck input on blur/disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusState {
    pub focused: bool,
    pub sequence: u64,
}

/// Explicit release message for pointer/keyboard cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAllInput {
    pub sequence: u64,
}
