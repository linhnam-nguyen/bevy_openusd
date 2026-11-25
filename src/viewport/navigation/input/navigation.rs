//! Shared navigation state for native windows and remote video surfaces.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use bevy::ecs::message::MessageCursor;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowEvent};
use viewport_protocol::{
    ButtonState, FocusState, InputCommand, InputModifiers, KeyboardInput, PointerButtons,
    PointerMotion,
};

use super::ViewerInteractionGate;
use crate::viewport::rendering::offscreen::HeadlessInputWindow;

const REMOTE_INPUT_TIMEOUT: Duration = Duration::from_millis(750);
const REMOTE_PAN_MULTIPLIER: f32 = 2.0;
const PRIMARY_EDGE_CAPACITY: usize = 64;

/// Native Bevy receives an ordered `WindowEvent` stream. `MouseButtonInput`
/// by itself contains no pointer position or modifiers, and the `Window`
/// resource stores only the latest cursor. Reconstruct button snapshots while
/// traversing cursor, keyboard and button events in their original order.
/// This state is local to the native-window adapter, not the remote or Mara host.
#[derive(Default)]
pub(crate) struct NativeOrderedInput {
    window: Option<Entity>,
    focused: bool,
    cursor: Option<Vec2>,
    modifier_keys: HashSet<KeyCode>,
}

impl NativeOrderedInput {
    fn reset_pointer(&mut self, input: &mut ViewportNavigationInput) {
        self.cursor = None;
        input.cancel_primary_input();
        input.buttons.primary = false;
    }

    fn modifiers(&self) -> InputModifiers {
        let pressed =
            |left, right| self.modifier_keys.contains(&left) || self.modifier_keys.contains(&right);
        InputModifiers {
            shift: pressed(KeyCode::ShiftLeft, KeyCode::ShiftRight),
            control: pressed(KeyCode::ControlLeft, KeyCode::ControlRight),
            alt: pressed(KeyCode::AltLeft, KeyCode::AltRight),
            meta: pressed(KeyCode::SuperLeft, KeyCode::SuperRight),
        }
    }

    fn update_key(&mut self, code: KeyCode, is_pressed: bool) {
        if !matches!(
            code,
            KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::ControlLeft
                | KeyCode::ControlRight
                | KeyCode::AltLeft
                | KeyCode::AltRight
                | KeyCode::SuperLeft
                | KeyCode::SuperRight
        ) {
            return;
        }
        if is_pressed {
            self.modifier_keys.insert(code);
        } else {
            self.modifier_keys.remove(&code);
        }
    }

    fn apply_event(
        &mut self,
        event: &WindowEvent,
        window: Entity,
        viewport: Vec2,
        enabled: bool,
        input: &mut ViewportNavigationInput,
    ) {
        match event {
            WindowEvent::CursorMoved(moved) if moved.window == window && self.focused => {
                self.cursor = moved.position.is_finite().then_some(moved.position);
            }
            WindowEvent::CursorLeft(left) if left.window == window => {
                self.reset_pointer(input);
            }
            WindowEvent::KeyboardInput(key) if key.window == window && self.focused => {
                self.update_key(key.key_code, key.state.is_pressed());
            }
            WindowEvent::KeyboardFocusLost(_) => {
                self.modifier_keys.clear();
                self.reset_pointer(input);
            }
            WindowEvent::WindowFocused(change) if change.window == window => {
                self.focused = change.focused;
                if !change.focused {
                    self.modifier_keys.clear();
                    self.reset_pointer(input);
                }
            }
            WindowEvent::WindowResized(change) if change.window == window => {
                self.reset_pointer(input);
            }
            WindowEvent::WindowScaleFactorChanged(change) if change.window == window => {
                self.reset_pointer(input);
            }
            WindowEvent::MouseButtonInput(edge)
                if enabled && edge.window == window && edge.button == MouseButton::Left =>
            {
                if !self.focused {
                    return;
                }
                let Some(position) = self.cursor.filter(|position| {
                    position.is_finite()
                        && position.cmpge(Vec2::ZERO).all()
                        && position.cmplt(viewport).all()
                }) else {
                    // Never interpret N unordered button edges at the final
                    // `Window::cursor_position` (the bug in F-C06-I5-01).
                    self.reset_pointer(input);
                    bevy::log::debug!(
                        target: "viewport::input",
                        "discarded native button edge without event-ordered cursor"
                    );
                    return;
                };
                input.record_primary_edge_at(
                    edge.state.is_pressed(),
                    Some(position),
                    self.modifiers(),
                );
            }
            _ => {}
        }
    }
}

/// Ordered, renderer-neutral button edge. The pointer location is sampled at
/// the edge, not from the last coalesced motion received in this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PrimaryButtonEdge {
    pub(crate) pressed: bool,
    pub(crate) position: Option<Vec2>,
    pub(crate) modifiers: InputModifiers,
    pub(crate) viewport_size: Vec2,
    pub(crate) generation: u64,
}

/// Host UI capture state (populated by Mara native host, `false` in headless).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostUiInputCapture {
    pub(crate) wants_pointer: bool,
    pub(crate) wants_keyboard: bool,
}

pub(crate) fn ui_wants_pointer_input(capture: Option<Res<HostUiInputCapture>>) -> bool {
    capture.is_some_and(|c| c.wants_pointer)
}

pub(crate) fn ui_wants_keyboard_input(capture: Option<Res<HostUiInputCapture>>) -> bool {
    capture.is_some_and(|c| c.wants_keyboard)
}

/// Renderer-neutral input consumed by the arcball camera.
#[derive(Resource, Debug)]
pub(crate) struct ViewportNavigationInput {
    pub(crate) pointer_delta: Vec2,
    pub(crate) wheel_delta: Vec2,
    pub(crate) buttons: PointerButtons,
    pub(crate) modifiers: InputModifiers,
    pub(crate) viewport_size: Vec2,
    pub(crate) pointer_position: Option<Vec2>,
    pub(crate) focused: bool,
    pub(crate) generation: u64,
    /// Remote video surfaces use a larger pan response because CSS pixels
    /// are not the same interaction scale as a native Bevy window. Native
    /// input resets this to `1.0` and is therefore unchanged.
    pub(crate) pan_multiplier: f32,
    primary_edges: VecDeque<PrimaryButtonEdge>,
    primary_edges_overflowed: bool,
    primary_epoch: u64,
    reliable_pointer: Option<PointerMotion>,
    last_input_sequence: u64,
    last_motion_sequence: u64,
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
            pointer_position: None,
            focused: true,
            generation: 0,
            pan_multiplier: 1.0,
            primary_edges: VecDeque::new(),
            primary_edges_overflowed: false,
            primary_epoch: 0,
            reliable_pointer: None,
            last_input_sequence: 0,
            last_motion_sequence: 0,
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

    /// Capture every primary edge in arrival order, including a whole tap
    /// delivered between two renderer Update invocations. Only one bounded
    /// queue is authoritative; button levels remain for camera navigation.
    pub(crate) fn record_primary_edge(&mut self, pressed: bool) {
        self.record_primary_edge_at(pressed, self.pointer_position, self.modifiers);
    }

    fn record_primary_edge_at(
        &mut self,
        pressed: bool,
        position: Option<Vec2>,
        modifiers: InputModifiers,
    ) {
        let previous = self
            .primary_edges
            .back()
            .map_or(self.buttons.primary, |edge| edge.pressed);
        if previous == pressed || self.primary_edges_overflowed {
            return;
        }
        if self.primary_edges.len() >= PRIMARY_EDGE_CAPACITY {
            self.primary_edges.clear();
            self.primary_edges_overflowed = true;
            bevy::log::warn!(target: "viewport::input", "primary button edge queue overflow; cancelling gesture");
            return;
        }
        self.primary_edges.push_back(PrimaryButtonEdge {
            pressed,
            position,
            modifiers,
            viewport_size: self.viewport_size,
            generation: self.generation,
        });
    }

    /// Draining is explicit: never clear edges with the per-frame motion reset.
    pub(crate) fn take_primary_edges(&mut self) -> (Vec<PrimaryButtonEdge>, bool) {
        let overflowed = std::mem::take(&mut self.primary_edges_overflowed);
        let edges = self.primary_edges.drain(..).collect();
        (edges, overflowed)
    }

    pub(crate) fn primary_epoch(&self) -> u64 {
        self.primary_epoch
    }

    fn cancel_primary_input(&mut self) {
        self.primary_edges.clear();
        self.primary_edges_overflowed = false;
        self.primary_epoch = self.primary_epoch.wrapping_add(1);
        self.reliable_pointer = None;
    }

    /// Begins a new authoritative stream coordinate mapping. Any movement or
    /// button state from the previous target must not affect the first frame
    /// of the new generation, so it is cleared before the client receives
    /// `ConfigurationApplied` and starts sending the matching generation.
    pub(crate) fn begin_stream_generation(&mut self, generation: u64) {
        if generation == 0 || self.generation == generation {
            return;
        }
        self.pointer_delta = Vec2::ZERO;
        self.wheel_delta = Vec2::ZERO;
        self.pointer_position = None;
        self.buttons = PointerButtons::default();
        self.cancel_primary_input();
        self.modifiers = InputModifiers::default();
        self.focused = false;
        self.generation = generation;
        self.pan_multiplier = 1.0;
        self.last_input_sequence = 0;
        self.last_motion_sequence = 0;
        self.remote_last_activity = None;
    }

    pub(crate) fn apply_remote_command(&mut self, command: InputCommand) {
        match command {
            InputCommand::PointerMotion(motion) => {
                if motion.sequence != 0 && self.accept_generation(motion.stream_generation) {
                    self.reliable_pointer = Some(motion);
                    self.apply_pointer_motion(motion);
                }
            }
            InputCommand::ButtonState(ButtonState {
                sequence,
                buttons,
                modifiers,
                stream_generation,
            }) => {
                if !self.accept_sequence(sequence) || !self.accept_generation(stream_generation) {
                    return;
                }
                self.modifiers = modifiers;
                self.focused = true;
                // Edge coordinates are independent of a newer unordered packet.
                let pointer = self.pointer_position;
                let viewport = self.viewport_size;
                if let Some(snapshot) = self.reliable_pointer.take() {
                    self.viewport_size =
                        Vec2::new(snapshot.viewport_css_width, snapshot.viewport_css_height)
                            .max(Vec2::ONE);
                    self.pointer_position = Some(
                        Vec2::new(snapshot.x_css_pixels, snapshot.y_css_pixels)
                            .clamp(Vec2::ZERO, self.viewport_size),
                    );
                }
                self.record_primary_edge(buttons.primary);
                self.pointer_position = pointer;
                self.viewport_size = viewport;
                self.buttons = buttons;
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
        self.pointer_delta = Vec2::new(motion.dx_css_pixels, motion.dy_css_pixels);
        self.wheel_delta = Vec2::new(motion.wheel_x, motion.wheel_y);
        self.pan_multiplier = REMOTE_PAN_MULTIPLIER;
        self.viewport_size = Vec2::new(
            motion.viewport_css_width.max(1.0),
            motion.viewport_css_height.max(1.0),
        );
        self.pointer_position = Some(
            Vec2::new(motion.x_css_pixels, motion.y_css_pixels)
                .clamp(Vec2::ZERO, self.viewport_size),
        );
        self.focused = true;
        self.note_remote_activity();
    }

    pub(crate) fn clear_remote_state(&mut self) {
        self.pointer_delta = Vec2::ZERO;
        self.wheel_delta = Vec2::ZERO;
        self.buttons = PointerButtons::default();
        self.cancel_primary_input();
        self.modifiers = InputModifiers::default();
        self.pointer_position = None;
        self.focused = false;
        self.generation = 0;
        self.pan_multiplier = 1.0;
        self.last_input_sequence = 0;
        self.last_motion_sequence = 0;
        self.remote_last_activity = None;
    }

    pub(crate) fn expire_remote_input(&mut self) {
        if self
            .remote_last_activity
            .is_some_and(|last| last.elapsed() >= REMOTE_INPUT_TIMEOUT)
        {
            // Explicit blur/disconnect resets the intent queue. The idle
            // watchdog only needs to release held input; a completed click may
            // still be traversing multiple GPU batches after input goes quiet.
            if self.buttons != PointerButtons::default() {
                self.clear_remote_state();
            } else {
                self.modifiers = InputModifiers::default();
                self.remote_last_activity = None;
            }
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
        if self.generation == 0 {
            self.generation = generation;
            return true;
        }
        self.generation == generation
    }

    fn note_remote_activity(&mut self) {
        self.remote_last_activity = Some(Instant::now());
    }
}

/// Clears only per-frame deltas; button/focus state persists until an
/// authoritative reliable release/focus message or the inactivity timeout.
pub(crate) fn reset_navigation_frame(mut input: ResMut<ViewportNavigationInput>) {
    input.reset_frame_motion();
}

/// Keeps the local native window behavior on the same camera input resource.
/// In headless mode the query is empty and the remote adapter is the only
/// producer.
pub(crate) fn apply_local_navigation_input(
    window: Query<(Entity, &Window), (With<PrimaryWindow>, Without<HeadlessInputWindow>)>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_events: Option<Res<Messages<WindowEvent>>>,
    mut window_event_cursor: Local<MessageCursor<WindowEvent>>,
    mut ordered_native: Local<NativeOrderedInput>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pan_anchor: Local<Option<Vec2>>,
    mut orbit_anchor: Local<Option<Vec2>>,
    mut input: ResMut<ViewportNavigationInput>,
    gate: ViewerInteractionGate,
) {
    let Ok((window_entity, window)) = window.single() else {
        if let Some(events) = window_events.as_deref() {
            window_event_cursor.clear(events);
        }
        if ordered_native.window.is_some() {
            *ordered_native = NativeOrderedInput::default();
            *pan_anchor = None;
            *orbit_anchor = None;
            input.clear_remote_state();
        }
        return;
    };
    if ordered_native.window != Some(window_entity) {
        // On first binding, a focus event in the unread batch gives the
        // entrance state. Later batches start from our prior observation.
        let focused = window_events
            .as_deref()
            .and_then(|events| {
                window_event_cursor
                    .clone()
                    .read(events)
                    .find_map(|event| match event {
                        WindowEvent::WindowFocused(change) if change.window == window_entity => {
                            Some(!change.focused)
                        }
                        _ => None,
                    })
            })
            .unwrap_or(window.focused);
        *ordered_native = NativeOrderedInput {
            window: Some(window_entity),
            focused,
            ..Default::default()
        };
        *pan_anchor = None;
        *orbit_anchor = None;
        input.clear_remote_state();
    }

    input.pan_multiplier = 1.0;
    input.viewport_size = Vec2::new(
        window.resolution.width().max(1.0),
        window.resolution.height().max(1.0),
    );
    input.pointer_position = window.cursor_position();
    let sampled_buttons = PointerButtons {
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
    // The combined WindowEvent stream preserves OS order across CursorMoved,
    // KeyboardInput and MouseButtonInput. Never sample the end-of-frame cursor
    // for more than one edge, even if press/release happened in one Update.
    let locked = gate.locked();
    if let Some(events) = window_events.as_deref() {
        for event in window_event_cursor.read(events) {
            ordered_native.apply_event(
                event,
                window_entity,
                input.viewport_size,
                !locked,
                &mut input,
            );
        }
    } else {
        // Headless/test applications may omit WindowEvent registration. It is
        // unsafe to reconstruct click positions from level-only ButtonInput.
        ordered_native.modifier_keys.clear();
        ordered_native.reset_pointer(&mut input);
    }
    if ordered_native.focused != window.focused {
        ordered_native.focused = window.focused;
        ordered_native.modifier_keys.clear();
        ordered_native.reset_pointer(&mut input);
    }
    if locked {
        ordered_native.cursor = None;
        *pan_anchor = None;
        *orbit_anchor = None;
        input.clear_remote_state();
        return;
    }
    input.focused = window.focused;
    input.buttons = sampled_buttons;

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

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/input/navigation_inline_tests.rs"]
mod tests;
