use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{SceneAnchor, ViewportCommand};

use super::{ActionQueue, PanelAction, push_action};
use crate::viewport::api::ViewportReadModelState;

#[derive(Clone)]
pub(super) struct SelectionSnapshot {
    stage_name: String,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
    primary: Option<SceneAnchor>,
    selection_count: usize,
    last_error: Option<String>,
}

pub(super) fn snapshot(world: &World) -> SelectionSnapshot {
    let rm = world.resource::<ViewportReadModelState>();
    let snap = rm.snapshot();
    let editor = rm.editor_state();
    SelectionSnapshot {
        stage_name: snap
            .map(|s| s.stage.display_name.clone())
            .unwrap_or_else(|| "(stage loading)".to_owned()),
        dirty: editor.is_dirty,
        can_undo: editor.can_undo,
        can_redo: editor.can_redo,
        primary: snap.and_then(|s| s.selection.primary.clone()),
        selection_count: snap.map(|s| s.selection.targets.len()).unwrap_or(0),
        last_error: rm.last_error().map(ToOwned::to_owned),
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &SelectionSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    if let Some(err) = &snap.last_error {
        let err_msg = err.clone();
        let q_dismiss = queue.clone();
        body.add_normal(
            "usd.error",
            "Last Error",
            "history",
            vec![
                Pod::new(MaraId::new(("mara.sel", "err_text"))).with_custom_units(2, move |ui| {
                    ui.label(&err_msg);
                    if ui.button("Dismiss").clicked() {
                        push_action(&q_dismiss, PanelAction::ClearErrorBanner);
                    }
                }),
            ],
        );
    }

    let q_open = queue.clone();
    let q_reveal = queue.clone();
    let q_save = queue.clone();
    let q_save_as = queue.clone();
    let q_reload = queue.clone();
    let q_undo = queue.clone();
    let q_redo = queue.clone();
    let dirty = snap.dirty;
    let can_undo = snap.can_undo;
    let can_redo = snap.can_redo;

    body.add_normal(
        "usd.stage_file",
        "Stage & File",
        "folder",
        vec![
            Pod::new(MaraId::new(("mara.sel", "file")))
                .with_readout("file", &snap.stage_name),
            Pod::new(MaraId::new(("mara.sel", "dirty")))
                .with_readout("modified", if dirty { "Yes" } else { "No" }),
            Pod::new(MaraId::new(("mara.sel", "file_actions"))).with_custom_units(5, move |ui| {
                if ui.button("Open USD…").clicked() {
                    push_action(&q_open, PanelAction::OpenUsdDialog);
                }
                if ui.button("Reveal in filesystem").clicked() {
                    push_action(&q_reveal, PanelAction::RevealInFilesystem);
                }
                if ui.button("Save stage").clicked() {
                    push_action(&q_save, PanelAction::Command(ViewportCommand::SaveStage));
                }
                if ui.button("Save stage as…").clicked() {
                    push_action(&q_save_as, PanelAction::SaveStageAsDialog);
                }
                if ui.button("Reload stage (R)").clicked() {
                    push_action(
                        &q_reload,
                        PanelAction::Command(ViewportCommand::ReloadSession),
                    );
                }
            }),
            Pod::new(MaraId::new(("mara.sel", "history_actions"))).with_custom_units(2, move |ui| {
                ui.horizontal(|ui| {
                    let undo_label = if can_undo { "Undo" } else { "Undo (—)" };
                    if ui.button(undo_label).clicked() && can_undo {
                        push_action(&q_undo, PanelAction::Command(ViewportCommand::UndoEditor));
                    }
                    let redo_label = if can_redo { "Redo" } else { "Redo (—)" };
                    if ui.button(redo_label).clicked() && can_redo {
                        push_action(&q_redo, PanelAction::Command(ViewportCommand::RedoEditor));
                    }
                });
            }),
        ],
    );

    let mut sel_pods = vec![
        Pod::new(MaraId::new(("mara.sel", "count")))
            .with_readout("selected", snap.selection_count.to_string()),
    ];
    if let Some(target) = &snap.primary {
        let name = target
            .prim_path
            .rsplit('/')
            .next()
            .unwrap_or(&target.prim_path)
            .to_owned();
        sel_pods.push(Pod::new(MaraId::new(("mara.sel", "name"))).with_readout("name", name));
        sel_pods.push(
            Pod::new(MaraId::new(("mara.sel", "path"))).with_readout("path", &target.prim_path),
        );
        if let Some(ctx) = &target.instance_context {
            sel_pods.push(Pod::new(MaraId::new(("mara.sel", "inst"))).with_readout("instance", ctx));
        }
        let prim_path = target.prim_path.clone();
        let q_clear = queue.clone();
        let q_load = queue.clone();
        let q_unload = queue.clone();
        sel_pods.push(
            Pod::new(MaraId::new(("mara.sel", "sel_btns"))).with_custom_units(3, move |ui| {
                if ui.button("Clear selection").clicked() {
                    push_action(&q_clear, PanelAction::Command(ViewportCommand::ClearSelection));
                }
                if ui.button("Load payload").clicked() {
                    push_action(
                        &q_load,
                        PanelAction::Command(ViewportCommand::LoadPayload {
                            prim_path: prim_path.clone(),
                        }),
                    );
                }
                if ui.button("Unload payload").clicked() {
                    push_action(
                        &q_unload,
                        PanelAction::Command(ViewportCommand::UnloadPayload {
                            prim_path: prim_path.clone(),
                        }),
                    );
                }
            }),
        );
    } else {
        sel_pods.push(
            Pod::new(MaraId::new(("mara.sel", "hint")))
                .with_readout("hint", "Click a prim in the Tree"),
        );
    }
    let _ = accent;
    body.add_normal("usd.selected_prim", "Selected Prim", "cube", sel_pods);
}
