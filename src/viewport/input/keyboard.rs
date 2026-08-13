//! Keyboard shortcuts: panel toggles + overlay toggles.
//!
//! Run-if `not(egui_wants_any_pointer_input)` so typing in a future search
//! field doesn't also toggle panels.

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use bevy_frost::RibbonOpen;

use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::ui_frost::{
    RIB_INFO, RIB_KEYS, RIB_OVERLAYS, RIB_TREE, RIBBON_LEFT, ViewerCommandPalette,
};
use viewport_protocol::{OverlayKind, ViewportCommand};

pub struct ViewerKeyboardPlugin;

impl Plugin for ViewerKeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                handle_keys.run_if(not(egui_wants_any_keyboard_input)),
                handle_palette_shortcut,
            ),
        );
    }
}

/// Ctrl+K / Ctrl+P opens or closes the command palette. Runs
/// unguarded by egui's keyboard grab so the shortcut works while
/// the palette itself has focus (closing it via the same chord).
fn handle_palette_shortcut(
    keys: Res<ButtonInput<KeyCode>>,
    mut palette: ResMut<ViewerCommandPalette>,
) {
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !ctrl {
        return;
    }
    if keys.just_pressed(KeyCode::KeyK) || keys.just_pressed(KeyCode::KeyP) {
        palette.0.open = !palette.0.open;
        if palette.0.open {
            palette.0.query.clear();
            palette.0.selected = 0;
        }
    }
}

/// Maps viewer hotkeys to ribbon panels, overlay flags, and reload requests.
fn handle_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut ribbon: ResMut<RibbonOpen>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if keys.just_pressed(KeyCode::KeyT) {
        ribbon.toggle(RIBBON_LEFT, RIB_TREE);
    }
    if keys.just_pressed(KeyCode::KeyI) {
        ribbon.toggle(RIBBON_LEFT, RIB_INFO);
    }
    if keys.just_pressed(KeyCode::KeyO) {
        ribbon.toggle(RIBBON_LEFT, RIB_OVERLAYS);
    }
    // Both `/` and `?` sit on the same physical key.
    if keys.just_pressed(KeyCode::Slash) {
        ribbon.toggle(RIBBON_LEFT, RIB_KEYS);
    }

    let Some(snapshot) = read_model.snapshot() else {
        return;
    };
    let presentation = &snapshot.presentation;

    if keys.just_pressed(KeyCode::KeyG) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::GroundGrid,
            enabled: !presentation.ground_grid,
        });
    }
    if keys.just_pressed(KeyCode::KeyX) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::WorldAxes,
            enabled: !presentation.world_axes,
        });
    }
    if keys.just_pressed(KeyCode::KeyP) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::PrimMarkers,
            enabled: !presentation.prim_markers,
        });
    }
    if keys.just_pressed(KeyCode::KeyB) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::Skeleton,
            enabled: !presentation.skeleton,
        });
    }
    if keys.just_pressed(KeyCode::KeyY) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::Physics,
            enabled: !presentation.physics,
        });
    }
    if keys.just_pressed(KeyCode::KeyC) {
        viewport_commands.send(ViewportCommand::SetOverlay {
            overlay: OverlayKind::Colliders,
            enabled: !presentation.colliders,
        });
    }
    if keys.just_pressed(KeyCode::KeyR) {
        viewport_commands.send(ViewportCommand::ReloadSession);
    }
}
