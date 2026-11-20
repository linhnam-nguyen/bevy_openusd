//! Mara native input forwarding and keyboard shortcuts.
//!
//! Forwards egui pointer/wheel/modifier events inside the Mara viewport rect
//! into the shared `ViewportNavigationInput` resource and updates
//! `HostUiInputCapture` so Bevy camera/keyboard handlers yield whenever Mara UI
//! widgets hold focus or pointer capture.

use bevy::prelude::*;
use mara::host::MaraHostCtx;
use viewport_protocol::{InputModifiers, OverlayKind, PointerButtons, ViewportCommand};

use super::{MaraHostUiState, panels};
use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::input::{HostUiInputCapture, ViewportNavigationInput};

/// Synchronizes `HostUiInputCapture` and forwards viewport pointer/scroll/button
/// input from Mara's `egui::Context` into `ViewportNavigationInput`.
pub(super) fn sync_viewport_input(
    egui_ctx: &egui::Context,
    viewport_rect: egui::Rect,
    world: Option<&mut World>,
    ui_state: &mut MaraHostUiState,
) {
    let Some(world) = world else {
        return;
    };

    let wants_pointer = egui_ctx.egui_wants_pointer_input();
    let wants_keyboard = egui_ctx.egui_wants_keyboard_input();

    if let Some(mut capture) = world.get_resource_mut::<HostUiInputCapture>() {
        capture.wants_pointer = wants_pointer;
        capture.wants_keyboard = wants_keyboard;
    }

    let (pointer_pos, primary, secondary, middle, scroll_y, modifiers, focused) =
        egui_ctx.input(|i| {
            let pos = i.pointer.interact_pos().or_else(|| i.pointer.hover_pos());
            (
                pos,
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.middle_down(),
                i.smooth_scroll_delta.y / 32.0,
                InputModifiers {
                    shift: i.modifiers.shift,
                    control: i.modifiers.ctrl,
                    alt: i.modifiers.alt,
                    meta: i.modifiers.mac_cmd || i.modifiers.command,
                },
                i.focused,
            )
        });

    let pointer_in_viewport = pointer_pos.is_some_and(|p| viewport_rect.contains(p));
    let any_drag_button = primary || secondary || middle;
    if !wants_pointer && pointer_in_viewport && any_drag_button {
        ui_state.viewport_drag_active = true;
    } else if !any_drag_button {
        ui_state.viewport_drag_active = false;
        ui_state.last_viewport_drag_pos = None;
    }

    let can_drive_viewport = !wants_pointer && (pointer_in_viewport || ui_state.viewport_drag_active);
    let ppp = egui_ctx.pixels_per_point().max(f32::EPSILON);

    let mut nav = world.resource_mut::<ViewportNavigationInput>();
    nav.focused = focused && can_drive_viewport;
    nav.pan_multiplier = 1.0;
    nav.viewport_size = Vec2::new(
        (viewport_rect.width() * ppp).max(1.0),
        (viewport_rect.height() * ppp).max(1.0),
    );
    nav.modifiers = modifiers;

    if can_drive_viewport {
        nav.buttons = PointerButtons {
            primary,
            secondary,
            auxiliary: middle,
        };
        if let Some(pos) = pointer_pos {
            nav.pointer_position = Some(Vec2::new(
                ((pos.x - viewport_rect.left()) * ppp).clamp(0.0, nav.viewport_size.x),
                ((pos.y - viewport_rect.top()) * ppp).clamp(0.0, nav.viewport_size.y),
            ));
            let delta = ui_state
                .last_viewport_drag_pos
                .filter(|_| ui_state.viewport_drag_active)
                .map_or(Vec2::ZERO, |prev| {
                    Vec2::new((pos.x - prev.x) * ppp, (pos.y - prev.y) * ppp)
                });
            ui_state.last_viewport_drag_pos = if ui_state.viewport_drag_active {
                Some(pos)
            } else {
                None
            };
            nav.pointer_delta = delta;
        } else {
            nav.pointer_delta = Vec2::ZERO;
        }
        nav.wheel_delta = Vec2::new(0.0, if pointer_in_viewport { scroll_y } else { 0.0 });
    } else {
        nav.buttons = PointerButtons::default();
        nav.pointer_delta = Vec2::ZERO;
        nav.wheel_delta = Vec2::ZERO;
    }
}

/// Processes native viewer hotkeys (`T`, `I`, `O`, `?`, `G`, `X`, `P`, `B`, `Y`,
/// `C`, `R`, `Ctrl+Z`, `Ctrl+Shift+Z`/`Ctrl+Y`), yielding when Mara UI text
/// inputs hold keyboard focus.
pub(super) fn handle_shortcuts(host: &MaraHostCtx<'_>, world: Option<&mut World>) {
    let egui_ctx = host.__internal_egui();
    if egui_ctx.egui_wants_keyboard_input() {
        return;
    }

    let (t, i_key, o, slash, g, x, p, b, y, c, r, ctrl_z, ctrl_y) = egui_ctx.input(|inp| {
        let ctrl = inp.modifiers.ctrl || inp.modifiers.command;
        let shift = inp.modifiers.shift;
        (
            !ctrl && inp.key_pressed(egui::Key::T),
            !ctrl && inp.key_pressed(egui::Key::I),
            !ctrl && inp.key_pressed(egui::Key::O),
            !ctrl && inp.key_pressed(egui::Key::Slash),
            !ctrl && inp.key_pressed(egui::Key::G),
            !ctrl && inp.key_pressed(egui::Key::X),
            !ctrl && inp.key_pressed(egui::Key::P),
            !ctrl && inp.key_pressed(egui::Key::B),
            !ctrl && inp.key_pressed(egui::Key::Y),
            !ctrl && inp.key_pressed(egui::Key::C),
            !ctrl && inp.key_pressed(egui::Key::R),
            ctrl && !shift && inp.key_pressed(egui::Key::Z),
            (ctrl && shift && inp.key_pressed(egui::Key::Z))
                || (ctrl && !shift && inp.key_pressed(egui::Key::Y)),
        )
    });

    if t {
        host.toggle_rail_pane(panels::RIBBON_LEFT, panels::PANE_TREE);
    }
    if i_key {
        host.toggle_rail_pane(panels::RIBBON_LEFT, panels::PANE_INFO);
    }
    if o {
        host.toggle_rail_pane(panels::RIBBON_LEFT, panels::PANE_OVERLAYS);
    }
    if slash {
        host.toggle_rail_pane(panels::RIBBON_LEFT, panels::PANE_KEYS);
    }

    let Some(world) = world else {
        return;
    };
    let presentation = world
        .get_resource::<ViewportReadModelState>()
        .and_then(|rm| rm.snapshot())
        .map(|s| s.presentation.clone());

    let mut commands = world.resource_mut::<ViewportCommandInbox>();
    if ctrl_z {
        commands.send(ViewportCommand::UndoEditor);
    }
    if ctrl_y {
        commands.send(ViewportCommand::RedoEditor);
    }
    if r {
        commands.send(ViewportCommand::ReloadSession);
    }
    if let Some(pres) = presentation {
        if g {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::GroundGrid,
                enabled: !pres.ground_grid,
            });
        }
        if x {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::WorldAxes,
                enabled: !pres.world_axes,
            });
        }
        if p {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::PrimMarkers,
                enabled: !pres.prim_markers,
            });
        }
        if b {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::Skeleton,
                enabled: !pres.skeleton,
            });
        }
        if y {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::Physics,
                enabled: !pres.physics,
            });
        }
        if c {
            commands.send(ViewportCommand::SetOverlay {
                overlay: OverlayKind::Colliders,
                enabled: !pres.colliders,
            });
        }
    }
}
