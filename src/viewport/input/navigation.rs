//! Shared navigation state for native windows and remote video surfaces.

use std::time::{Duration, Instant};

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use viewport_protocol::{
    ButtonState, FocusState, InputCommand, InputModifiers, KeyboardInput, PointerButtons,
    PointerMotion,
};

const REMOTE_INPUT_TIMEOUT: Duration = Duration::from_millis(750);
const REMOTE_PAN_MULTIPLIER: f32 = 2.0;

/// Renderer-neutral input consumed by the arcball camera.
#[derive(Resource, Debug)]
pub(crate) struct ViewportNavigationInput {
    pub(crate) pointer_delta: Vec2,
    pub(crate) wheel_delta: Vec2,
    pub(crate) buttons: PointerButtons,
    pub(crate) modifiers: InputModifiers,
    pub(crate) viewport_size: Vec2,
    pub(crate) focused: bool,
    pub(crate) generation: u64,
    /// Remote video surfaces use a larger pan response because CSS pixels
    /// are not the same interaction scale as a native Bevy window. Native
    /// input resets this to `1.0` and is therefore unchanged.
    pub(crate) pan_multiplier: f32,
    last_input_sequence: u64,
    last_motion_sequence: u64,
    last_remote_motion_at: Option<Instant>,
    remote_last_activity: Option<Instant>,
}

impl Default for ViewportNavigationInput {
    fn default() -> Self {
        Self {
            pointer_delta: Vec2::ZERO,
            wheel_delta: Vec2::ZERO,
            buttons: PointerButtons::default(),
            modifiers: InputModifiers::default(),
            viewport_size: Vec2::new(1400.0, 900.0),
            focused: true,
            generation: 0,
            pan_multiplier: 1.0,
            last_input_sequence: 0,
            last_motion_sequence: 0,
            last_remote_motion_at: None,
            remote_last_activity: None,
        }
    }
}

impl ViewportNavigationInput {
    pub(crate) fn with_viewport_size(width: u32, height: u32) -> Self {
        Self {
            viewport_size: Vec2::new(width.max(1) as f32, height.max(1) as f32),
            ..Default::default()
        }
    }

    pub(crate) fn reset_frame_motion(&mut self) {
        self.pointer_delta = Vec2::ZERO;
        self.wheel_delta = Vec2::ZERO;
    }

    pub(crate) fn apply_remote_command(&mut self, command: InputCommand) {
        match command {
            InputCommand::PointerMotion(motion) => self.apply_pointer_motion(motion),
            InputCommand::ButtonState(ButtonState {
                sequence,
                buttons,
                modifiers,
                stream_generation,
            }) => {
                if !self.accept_sequence(sequence) || !self.accept_generation(stream_generation) {
                    return;
                }
                self.buttons = buttons;
                self.modifiers = modifiers;
                self.focused = true;
                self.pan_multiplier = REMOTE_PAN_MULTIPLIER;
                self.note_remote_activity();
            }
            InputCommand::Keyboard(KeyboardInput {
                sequence,
                modifiers,
                stream_generation,
                ..
            }) => {
                if !self.accept_sequence(sequence) || !self.accept_generation(stream_generation) {
                    return;
                }
                self.modifiers = modifiers;
                self.focused = true;
                self.note_remote_activity();
            }
            InputCommand::FocusChanged(FocusState { focused, sequence }) => {
                if !self.accept_sequence(sequence) {
                    return;
                }
                self.focused = focused;
                if focused {
                    self.note_remote_activity();
                } else {
                    self.clear_remote_state();
                }
            }
            InputCommand::ReleaseAll(release) => {
                if self.accept_sequence(release.sequence) {
                    self.clear_remote_state();
                }
            }
            InputCommand::SetModifiers(modifiers) => {
                self.modifiers = modifiers;
                self.note_remote_activity();
            }
        }
    }

    pub(crate) fn apply_pointer_motion(&mut self, motion: PointerMotion) {
        if motion.sequence == 0 || !self.accept_generation(motion.stream_generation) {
            return;
        }
        if motion.sequence <= self.last_motion_sequence {
            return;
        }
        self.last_motion_sequence = motion.sequence;
        self.last_remote_motion_at = Some(Instant::now());
        bevy::log::debug!(
            "[viewport-input] applied remote motion sequence {}",
            motion.sequence
        );
        self.pointer_delta = Vec2::new(motion.dx_css_pixels, motion.dy_css_pixels);
        self.wheel_delta = Vec2::new(motion.wheel_x, motion.wheel_y);
        self.pan_multiplier = REMOTE_PAN_MULTIPLIER;
        self.viewport_size = Vec2::new(
            motion.viewport_css_width.max(1.0),
            motion.viewport_css_height.max(1.0),
        );
        self.focused = true;
        self.note_remote_activity();
    }

    pub(crate) fn clear_remote_state(&mut self) {
        self.pointer_delta = Vec2::ZERO;
        self.wheel_delta = Vec2::ZERO;
        self.buttons = PointerButtons::default();
        self.modifiers = InputModifiers::default();
        self.focused = false;
        self.generation = 0;
        self.pan_multiplier = 1.0;
        self.last_input_sequence = 0;
        self.last_motion_sequence = 0;
        self.last_remote_motion_at = None;
        self.remote_last_activity = None;
    }

    pub(crate) fn expire_remote_input(&mut self) {
        if self
            .remote_last_activity
            .is_some_and(|last| last.elapsed() >= REMOTE_INPUT_TIMEOUT)
        {
            self.clear_remote_state();
        }
    }

    fn accept_sequence(&mut self, sequence: u64) -> bool {
        if sequence == 0 || sequence <= self.last_input_sequence {
            return false;
        }
        self.last_input_sequence = sequence;
        true
    }

    fn accept_generation(&mut self, generation: u64) -> bool {
        if generation == 0 {
            return true;
        }
        if self.generation == 0 {
            self.generation = generation;
            return true;
        }
        self.generation == generation
    }

    fn note_remote_activity(&mut self) {
        self.remote_last_activity = Some(Instant::now());
    }

    pub(crate) fn latest_remote_motion(&self) -> Option<(u64, Instant)> {
        self.last_remote_motion_at
            .map(|applied_at| (self.last_motion_sequence, applied_at))
    }
}

/// Clears only per-frame deltas; button/focus state persists until an
/// authoritative reliable release/focus message or the inactivity timeout.
pub(crate) fn reset_navigation_frame(mut input: ResMut<ViewportNavigationInput>) {
    input.reset_frame_motion();
}

/// Applies validated input received by the WebRTC transport. The transport
/// never mutates Bevy state directly; it only feeds this bounded adapter.
pub(crate) fn apply_remote_navigation_input(
    interface: Option<Res<crate::viewport::api::RenderServerInterface>>,
    mut input: ResMut<ViewportNavigationInput>,
) {
    let Some(interface) = interface else {
        return;
    };
    if interface.take_input_reset() {
        input.clear_remote_state();
    }
    while let Some(command) = interface.pop_input() {
        input.apply_remote_command(command);
    }
    if let Some(motion) = interface.take_latest_pointer_motion() {
        input.apply_pointer_motion(motion);
    }
    input.expire_remote_input();
}

/// Keeps the local native window behavior on the same camera input resource.
/// In headless mode the query is empty and the remote adapter is the only
/// producer.
pub(crate) fn apply_local_navigation_input(
    window: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pan_anchor: Local<Option<Vec2>>,
    mut orbit_anchor: Local<Option<Vec2>>,
    mut input: ResMut<ViewportNavigationInput>,
) {
    let Ok(window) = window.single() else {
        return;
    };

    input.focused = window.focused;
    input.pan_multiplier = 1.0;
    input.viewport_size = Vec2::new(
        window.resolution.width().max(1.0),
        window.resolution.height().max(1.0),
    );
    input.buttons = PointerButtons {
        primary: mouse_buttons.pressed(MouseButton::Left),
        secondary: mouse_buttons.pressed(MouseButton::Right),
        auxiliary: mouse_buttons.pressed(MouseButton::Middle),
    };
    input.modifiers = InputModifiers {
        shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        control: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        alt: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        meta: keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]),
    };

    let pan_drag = !input.modifiers.control
        && (input.buttons.auxiliary
            || (input.modifiers.shift && (input.buttons.primary || input.buttons.secondary)));
    let orbit_drag = !input.modifiers.shift
        && !input.modifiers.control
        && input.buttons.primary
        && input.buttons.secondary;
    let cursor = window.cursor_position();

    if !pan_drag {
        *pan_anchor = None;
    }
    if !orbit_drag {
        *orbit_anchor = None;
    }

    if let Some(cursor) = cursor {
        if pan_drag {
            input.pointer_delta = pan_anchor
                .as_ref()
                .map_or(Vec2::ZERO, |previous| cursor - *previous);
            *pan_anchor = Some(cursor);
        } else if orbit_drag {
            input.pointer_delta = orbit_anchor
                .as_ref()
                .map_or(Vec2::ZERO, |previous| cursor - *previous);
            *orbit_anchor = Some(cursor);
        }
    }

    input.wheel_delta.y = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 32.0,
    };
}
