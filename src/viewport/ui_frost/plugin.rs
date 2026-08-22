use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use bevy_frost::prelude::*;
use viewport_protocol::ViewportCommand;

use super::constants::{
    RIB_PLAY, RIBBON_ITEMS, RIBBON_LEFT, RIBBONS, TreeExpanded, TreeFilter, ViewerCommandPalette,
};
use super::panels::{
    cameras::draw_cameras_panel,
    info::draw_info_panel,
    log::draw_log_panel,
    materials::draw_materials_panel,
    overlays::draw_overlays_panel,
    palette::draw_palette_panel,
    selection::draw_selection_panel,
    timeline::{draw_keys_panel, draw_timeline_panel},
    variants::draw_variants_panel,
};
use super::tree;
use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy_frost::FrostPlugin>() {
            app.add_plugins(bevy_frost::FrostPlugin);
        }
        app.init_resource::<TreeExpanded>()
            .init_resource::<TreeFilter>()
            .init_resource::<tree::ProtocolTreeExpanded>()
            .init_resource::<ViewerCommandPalette>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    draw_ribbons,
                    draw_selection_panel,
                    tree::draw_tree_panel,
                    draw_info_panel,
                    draw_variants_panel,
                    draw_cameras_panel,
                    draw_materials_panel,
                    draw_overlays_panel,
                    draw_timeline_panel,
                    draw_keys_panel,
                    draw_log_panel,
                    draw_palette_panel,
                )
                    .chain(),
            );
    }
}

/// Draws the activity ribbon and sends its physics action through the public
/// command path. Its active state is reduced from authoritative events.
fn draw_ribbons(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
    read_model: Res<ViewportReadModelState>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let physics_on = read_model
        .snapshot()
        .is_some_and(|snapshot| snapshot.physics_running);
    let clicks = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        |id| id == RIB_PLAY && physics_on,
    );
    for click in clicks {
        if click.item == RIB_PLAY {
            viewport_commands.send(ViewportCommand::SetPhysicsRunning {
                running: !physics_on,
            });
        }
    }
}

/// Tests whether an item in the viewer's left ribbon currently owns a panel.
pub(in crate::viewport::ui_frost) fn is_panel_open(open: &RibbonOpen, item: &'static str) -> bool {
    open.is_open(RIBBON_LEFT, item)
}
