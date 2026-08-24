//! Shared navigation state for native windows and remote video surfaces.

use std::time::{Duration, Instant};

use bevy::camera::{ImageRenderTarget, NormalizedRenderTarget};
use bevy::ecs::message::MessageWriter;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_glacial::gizmo::{GizmoDragStarted, GizmoDragging};
use viewport_protocol::{
    ButtonState, FocusState, InputCommand, InputModifiers, KeyboardInput, PointerButtons,
    PointerMotion,
};

use crate::viewport::app::headless::{HeadlessInputWindow, OffscreenTarget};

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
    pub(crate) pointer_position: Option<Vec2>,
    pub(crate) focused: bool,
    pub(crate) generation: u64,
    /// Remote video surfaces use a larger pan response because CSS pixels
    /// are not the same interaction scale as a native Bevy window. Native
    /// input resets this to `1.0` and is therefore unchanged.
    pub(crate) pan_multiplier: f32,
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

/// Applies validated input received by the WebRTC transport. The transport
/// never mutates Bevy state directly; it only feeds this bounded adapter.
pub(crate) fn apply_remote_navigation_input(
    interface: Option<Res<crate::viewport::api::RenderServerInterface>>,
    mut input: ResMut<ViewportNavigationInput>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    let Some(interface) = interface else {
        return;
    };
    if interface.take_input_reset() {
        input.clear_remote_state();
    }
    while let Some(command) = interface.pop_input() {
        if let Some(ref mut c) = counters {
            c.remote_inputs_applied += 1;
        }
        input.apply_remote_command(command);
    }
    if let Some(motion) = interface.take_latest_pointer_motion() {
        if let Some(ref mut c) = counters {
            c.remote_inputs_applied += 1;
        }
        input.apply_pointer_motion(motion);
    }
    input.expire_remote_input();
}

/// Keeps the local native window behavior on the same camera input resource.
/// In headless mode the query is empty and the remote adapter is the only
/// producer.
pub(crate) fn apply_local_navigation_input(
    window: Query<&Window, (With<PrimaryWindow>, Without<HeadlessInputWindow>)>,
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
    input.pointer_position = window.cursor_position();
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

#[derive(Default)]
pub(crate) struct HeadlessPointerState {
    position: Option<Vec2>,
    buttons: PointerButtons,
    target: Option<Handle<Image>>,
    generation: u64,
}

/// Bridges the remote viewport input resource into Bevy's existing pointer
/// and Glacial drag protocols. The synthetic window is only a cursor/window
/// compatibility surface; picking and gizmo hit testing remain authoritative.
pub(crate) fn sync_headless_gizmo_input(
    target: Option<Res<OffscreenTarget>>,
    input: Res<ViewportNavigationInput>,
    mut window: Query<&mut Window, With<HeadlessInputWindow>>,
    mut pointer_events: MessageWriter<PointerInput>,
    mut drag_started: MessageWriter<GizmoDragStarted>,
    mut dragging: MessageWriter<GizmoDragging>,
    mut pointer: Local<HeadlessPointerState>,
) {
    let Some(target) = target else {
        return;
    };
    let Ok(mut window) = window.single_mut() else {
        return;
    };

    let target_changed = pointer.target.as_ref() != Some(&target.image_handle)
        || pointer.generation != target.generation;
    if target_changed {
        pointer.position = None;
        pointer.buttons = PointerButtons::default();
    }

    let width = input.viewport_size.x.max(1.0).round() as u32;
    let height = input.viewport_size.y.max(1.0).round() as u32;
    if window.resolution.physical_size() != UVec2::new(width, height) {
        window.resolution.set_physical_resolution(width, height);
    }
    window.focused = input.focused;
    window.set_cursor_position(input.pointer_position);

    let current_buttons = if input.focused {
        input.buttons
    } else {
        PointerButtons::default()
    };
    let current_position = input
        .pointer_position
        .or(pointer.position)
        .unwrap_or(Vec2::ZERO);
    let location = || Location {
        target: NormalizedRenderTarget::Image(ImageRenderTarget {
            handle: target.image_handle.clone(),
            scale_factor: 1.0,
        }),
        position: current_position,
    };

    if input.focused {
        if let Some(position) = input.pointer_position {
            let delta = pointer
                .position
                .map_or(Vec2::ZERO, |previous| position - previous);
            if target_changed || pointer.position != Some(position) {
                pointer_events.write(PointerInput::new(
                    PointerId::Mouse,
                    location(),
                    PointerAction::Move { delta },
                ));
            }
            pointer.position = Some(position);
        }
    } else {
        pointer.position = None;
    }

    emit_pointer_button_transitions(
        pointer.buttons,
        current_buttons,
        &location,
        &mut pointer_events,
    );

    if !pointer.buttons.primary && current_buttons.primary {
        drag_started.write_default();
    }
    if current_buttons.primary {
        dragging.write_default();
    }

    pointer.buttons = current_buttons;
    pointer.target = Some(target.image_handle.clone());
    pointer.generation = target.generation;
}

fn emit_pointer_button_transitions(
    previous: PointerButtons,
    current: PointerButtons,
    location: &impl Fn() -> Location,
    pointer_events: &mut MessageWriter<PointerInput>,
) {
    let transitions = [
        (previous.primary, current.primary, PointerButton::Primary),
        (
            previous.secondary,
            current.secondary,
            PointerButton::Secondary,
        ),
        (previous.auxiliary, current.auxiliary, PointerButton::Middle),
    ];
    for (was_pressed, is_pressed, button) in transitions {
        let Some(transition) = pointer_button_transition(was_pressed, is_pressed) else {
            continue;
        };
        let action = match transition {
            PointerTransition::Press => PointerAction::Press(button),
            PointerTransition::Release => PointerAction::Release(button),
        };
        pointer_events.write(PointerInput::new(PointerId::Mouse, location(), action));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerTransition {
    Press,
    Release,
}

fn pointer_button_transition(was_pressed: bool, is_pressed: bool) -> Option<PointerTransition> {
    match (was_pressed, is_pressed) {
        (false, true) => Some(PointerTransition::Press),
        (true, false) => Some(PointerTransition::Release),
        (false, false) | (true, true) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stream_generation_rejects_old_pointer_mapping() {
        let mut input = ViewportNavigationInput::with_viewport_size(1280, 720);
        input.begin_stream_generation(4);
        input.apply_pointer_motion(PointerMotion {
            sequence: 1,
            x_css_pixels: 40.0,
            y_css_pixels: 30.0,
            dx_css_pixels: 12.0,
            dy_css_pixels: 8.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 1280.0,
            viewport_css_height: 720.0,
            stream_generation: 3,
        });
        assert_eq!(input.pointer_delta, Vec2::ZERO);

        input.apply_pointer_motion(PointerMotion {
            sequence: 2,
            x_css_pixels: 48.0,
            y_css_pixels: 36.0,
            dx_css_pixels: 12.0,
            dy_css_pixels: 8.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 1280.0,
            viewport_css_height: 720.0,
            stream_generation: 4,
        });
        assert_eq!(input.pointer_delta, Vec2::new(12.0, 8.0));
    }

    #[test]
    fn remote_motion_uses_authoritative_viewport_cursor_without_publishing_state() {
        let mut input = ViewportNavigationInput::with_viewport_size(100, 80);
        input.apply_pointer_motion(PointerMotion {
            sequence: 1,
            x_css_pixels: 17.0,
            y_css_pixels: 63.0,
            dx_css_pixels: 10.0,
            dy_css_pixels: -5.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 100.0,
            viewport_css_height: 80.0,
            stream_generation: 1,
        });
        assert_eq!(input.pointer_position, Some(Vec2::new(17.0, 63.0)));
    }

    #[test]
    fn pointer_button_transitions_cover_press_release_and_middle_mapping() {
        let previous = PointerButtons {
            primary: true,
            secondary: false,
            auxiliary: true,
        };
        let current = PointerButtons {
            primary: false,
            secondary: true,
            auxiliary: false,
        };

        assert_eq!(
            pointer_button_transition(previous.primary, current.primary),
            Some(PointerTransition::Release)
        );
        assert_eq!(
            pointer_button_transition(previous.secondary, current.secondary),
            Some(PointerTransition::Press)
        );
        assert_eq!(
            pointer_button_transition(previous.auxiliary, current.auxiliary),
            Some(PointerTransition::Release)
        );
    }
}
