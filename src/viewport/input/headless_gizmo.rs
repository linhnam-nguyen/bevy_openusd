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

/// Maps a remote browser CSS-pixel coordinate into the physical pixels of the
/// offscreen image that owns picking and gizmo hit testing.
fn normalize_pointer_position(position: Vec2, source_size: Vec2, target_size: UVec2) -> Vec2 {
    let source_size = source_size.max(Vec2::ONE);
    let target_size = Vec2::new(target_size.x.max(1) as f32, target_size.y.max(1) as f32);
    (position / source_size * target_size).clamp(Vec2::ZERO, target_size)
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

    let target_size = UVec2::new(target.width.max(1), target.height.max(1));
    if window.resolution.physical_size() != target_size {
        window
            .resolution
            .set_physical_resolution(target_size.x, target_size.y);
    }
    window.focused = input.focused;
    let normalized_position = input
        .pointer_position
        .map(|position| normalize_pointer_position(position, input.viewport_size, target_size));
    window.set_cursor_position(normalized_position);

    let current_buttons = if input.focused {
        input.buttons
    } else {
        PointerButtons::default()
    };
    let current_position = input
        .pointer_position
        .map(|position| normalize_pointer_position(position, input.viewport_size, target_size))
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

    #[test]
    fn remote_css_pointer_maps_to_two_x_offscreen_target() {
        let mapped = normalize_pointer_position(
            Vec2::new(480.0, 270.0),
            Vec2::new(960.0, 540.0),
            UVec2::new(1920, 1080),
        );

        assert!(mapped.abs_diff_eq(Vec2::new(960.0, 540.0), 1e-5));
    }

    #[test]
    fn remote_css_pointer_maps_to_non_two_x_offscreen_target() {
        let mapped = normalize_pointer_position(
            Vec2::new(320.0, 180.0),
            Vec2::new(1280.0, 720.0),
            UVec2::new(1920, 1080),
        );

        assert!(mapped.abs_diff_eq(Vec2::new(480.0, 270.0), 1e-5));
    }

    #[test]
    fn remote_css_pointer_is_clamped_to_offscreen_target() {
        let mapped = normalize_pointer_position(
            Vec2::new(1400.0, -20.0),
            Vec2::new(1280.0, 720.0),
            UVec2::new(1920, 1080),
        );

        assert_eq!(mapped, Vec2::new(1920.0, 0.0));
    }
}
