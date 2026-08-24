//! Remote pointer and drag adaptation for the offscreen Glacial gizmo path.

use bevy::camera::{ImageRenderTarget, NormalizedRenderTarget};
use bevy::ecs::message::MessageWriter;
use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};
use bevy::prelude::*;
use bevy_glacial::gizmo::{GizmoDragStarted, GizmoDragging};
use viewport_protocol::PointerButtons;

use crate::viewport::app::headless::{HeadlessInputWindow, OffscreenTarget};
use crate::viewport::input::ViewportNavigationInput;

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
