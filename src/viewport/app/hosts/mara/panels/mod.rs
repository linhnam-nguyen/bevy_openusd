//! Mara native presentation panels and action dispatch.
//!
//! Every panel renders from `ViewportReadModelState` and backend read-only state
//! on the single Bevy `World`, queuing all mutations as `PanelAction`s routed
//! through `ViewportCommandInbox` and existing backend resources.

pub(super) mod bim;
pub(super) mod cameras;
pub(super) mod info;
pub(super) mod materials;
pub(super) mod overlays;
pub(super) mod selection;
pub(super) mod timeline;
pub(super) mod tree;
pub(super) mod variants;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use bevy::prelude::*;
use mara::host::{MaraHostCtx, RibbonRail};
use mara::ui::mara_core;
use mara_core::pane::PaneAnchor;
use mara_core::pane::RailZone;
use mara_core::ribbon::{RibbonAction, RibbonCluster, RibbonSlotClick};
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{BimPropertyGroupId, SceneAnchor, ViewportCommand};

use super::{MaraHostUiState, PendingFileDialog};
use crate::viewport::api::{ViewportCommandInbox, ViewportReadModelState};
use crate::viewport::camera::{
    ArcballCamera, CameraBookmark, CameraBookmarks, CameraMount, FlyTo, sync_pan_orbit_from_arcball,
};
use crate::viewport::diagnostics::log_capture::LoaderLog;
use crate::viewport::semantic::SemanticDiffState;
use crate::viewport::stage::{LoadRequest, StageInfo};

pub(super) const RIBBON_LEFT: &str = "usdview_mara_left";

pub(super) const PANE_SELECTION: &str = "usdview_pane_selection";
pub(super) const PANE_TREE: &str = "usdview_pane_tree";
pub(super) const PANE_INFO: &str = "usdview_pane_info";
pub(super) const PANE_BIM: &str = "usdview_pane_bim";
pub(super) const PANE_VARIANTS: &str = "usdview_pane_variants";
pub(super) const PANE_CAMERAS: &str = "usdview_pane_cameras";
pub(super) const PANE_MATERIALS: &str = "usdview_pane_materials";
pub(super) const PANE_OVERLAYS: &str = "usdview_pane_overlays";
pub(super) const PANE_TIMELINE: &str = "usdview_pane_timeline";
pub(super) const PANE_KEYS: &str = "usdview_pane_keys";
pub(super) const PANE_LOG: &str = "usdview_pane_log";

const ACTION_OPEN: &str = "usdview_action_open";
const ACTION_SAVE: &str = "usdview_action_save";
const ACTION_RELOAD: &str = "usdview_action_reload";
const ACTION_UNDO: &str = "usdview_action_undo";
const ACTION_REDO: &str = "usdview_action_redo";
const ACTION_FRAME: &str = "usdview_action_frame";
const ACTION_PLAY_PHYSICS: &str = "usdview_action_play_physics";

pub(crate) const ICON_SELECTION: &str = "folder";
pub(crate) const ICON_TREE: &str = "cube-tree";
pub(crate) const ICON_INFO: &str = "list";
pub(crate) const ICON_BIM: &str = "options";
pub(crate) const ICON_VARIANTS: &str = "branch";
pub(crate) const ICON_CAMERAS: &str = "cube";
pub(crate) const ICON_MATERIALS: &str = "color";
pub(crate) const ICON_OVERLAYS: &str = "eye";
pub(crate) const ICON_TIMELINE: &str = "clock";
pub(crate) const ICON_KEYS: &str = "keyboard";
pub(crate) const ICON_LOG: &str = "history";
pub(crate) const ICON_OPEN: &str = "folder-open";
pub(crate) const ICON_SAVE: &str = "save";
pub(crate) const ICON_RELOAD: &str = "image-arrow-counterclockwise";
pub(crate) const ICON_UNDO: &str = "arrow-undo";
pub(crate) const ICON_REDO: &str = "arrow-redo";
pub(crate) const ICON_FRAME: &str = "scan";
pub(crate) const ICON_PHYSICS: &str = "box";

pub(crate) const MARA_ICONS: &[&str] = &[
    ICON_SELECTION,
    ICON_TREE,
    ICON_INFO,
    ICON_BIM,
    ICON_VARIANTS,
    ICON_CAMERAS,
    ICON_MATERIALS,
    ICON_OVERLAYS,
    ICON_TIMELINE,
    ICON_KEYS,
    ICON_LOG,
    ICON_OPEN,
    ICON_SAVE,
    ICON_RELOAD,
    ICON_UNDO,
    ICON_REDO,
    ICON_FRAME,
    ICON_PHYSICS,
];

fn ribbon_action(id: &'static str) -> RibbonAction {
    RibbonAction::Command(MaraId::new(id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::viewport::app::hosts::mara) struct PageWindow {
    pub(in crate::viewport::app::hosts::mara) page: usize,
    pub(in crate::viewport::app::hosts::mara) total_pages: usize,
    pub(in crate::viewport::app::hosts::mara) start: usize,
    pub(in crate::viewport::app::hosts::mara) end: usize,
    pub(in crate::viewport::app::hosts::mara) total: usize,
}

impl PageWindow {
    pub(in crate::viewport::app::hosts::mara) fn range_label(self) -> String {
        if self.total == 0 {
            "0 of 0".to_owned()
        } else {
            format!("{}–{} of {}", self.start + 1, self.end, self.total)
        }
    }
}

pub(in crate::viewport::app::hosts::mara) fn page_window(
    total: usize,
    desired_page: usize,
    page_size: usize,
) -> PageWindow {
    assert!(page_size > 0, "page size must be nonzero");
    let total_pages = (total / page_size) + usize::from(total % page_size != 0);
    let last = total_pages.saturating_sub(1);
    let page = desired_page.min(last);
    let start = page.saturating_mul(page_size).min(total);
    let end = start.saturating_add(page_size).min(total);
    PageWindow {
        page,
        total_pages,
        start,
        end,
        total,
    }
}

/// Actions emitted inside `Pod` / `TreeBody` closures and flushed to the
/// single authoritative Bevy `World` immediately after `show_ribbon_rail`.
#[derive(Clone)]
pub(super) enum PanelAction {
    Command(ViewportCommand),
    OpenUsdDialog,
    SaveStageAsDialog,
    RevealInFilesystem,
    ClearErrorBanner,
    CaptureDiffBaseline,
    ClearDiffBaseline,
    ClearLog,
    SaveBookmark,
    JumpBookmark(usize),
    DeleteBookmark(usize),
    FrameVisibleScene,
    TreeSearch(String),
    TreeLoadMoreSearch,
    TreeExpand(SceneAnchor, bool),
    TreeExpandAllLoaded(Vec<SceneAnchor>),
    SetCataloguePage(usize),
    SetPropertyPage {
        group_id: BimPropertyGroupId,
        page: usize,
    },
    SetMaterialPage(usize),
}

pub(super) type ActionQueue = Arc<Mutex<Vec<PanelAction>>>;

pub(super) fn push_action(queue: &ActionQueue, action: PanelAction) {
    if let Ok(mut guard) = queue.lock() {
        guard.push(action);
    }
}

pub(super) fn flush_pending_catalogue_refresh(world: &mut World) {
    let has_inbox = world.contains_resource::<ViewportCommandInbox>();
    let should_dispatch = {
        let Some(mut rm) = world.get_resource_mut::<ViewportReadModelState>() else {
            return;
        };
        if !has_inbox {
            rm.clear_catalogue_refresh_needed();
            return;
        }
        rm.take_catalogue_refresh_for_current_stage()
    };
    if !should_dispatch {
        return;
    }
    let request_id = world.resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::RequestBimClassificationFieldCatalogue {
            known_revision: None,
        },
    );
    let _ = world
        .resource_mut::<ViewportReadModelState>()
        .begin_bim_catalogue_request(request_id);
}

pub(super) fn show_panels(
    host: &mut MaraHostCtx<'_>,
    world: &mut World,
    ui_state: &mut MaraHostUiState,
    accent: MaraColor32,
) {
    flush_pending_catalogue_refresh(world);
    ui_state.pages.sync_contexts(world);
    let queue: ActionQueue = Arc::new(Mutex::new(Vec::new()));
    let rail = build_rail(world, ui_state, accent, &queue);
    let clicks = host.show_ribbon_rail(rail, accent);
    handle_rail_actions(clicks, world, ui_state);

    if let Ok(mut pending) = queue.lock() {
        for action in pending.drain(..) {
            apply_panel_action(action, world, ui_state);
        }
    }
}

fn build_rail<'a>(
    world: &World,
    ui_state: &MaraHostUiState,
    accent: MaraColor32,
    queue: &ActionQueue,
) -> RibbonRail<'a, 'a> {
    let selection_snap = selection::snapshot(world);
    let tree_snap = tree::snapshot(world, &ui_state.tree);
    let info_snap = info::snapshot(world);
    let bim_snap = bim::snapshot(world, &ui_state.pages);
    let variants_snap = variants::snapshot(world);
    let cameras_snap = cameras::snapshot(world);
    let materials_snap = materials::snapshot(world, &ui_state.pages);
    let overlays_snap = overlays::snapshot(world);
    let timeline_snap = timeline::snapshot(world);
    let log_snap = timeline::log_snapshot(world);

    let q_sel = Arc::clone(queue);
    let q_tree = Arc::clone(queue);
    let q_info = Arc::clone(queue);
    let q_bim = Arc::clone(queue);
    let q_var = Arc::clone(queue);
    let q_cam = Arc::clone(queue);
    let q_mat = Arc::clone(queue);
    let q_ovl = Arc::clone(queue);
    let q_tl = Arc::clone(queue);
    let q_log = Arc::clone(queue);

    RibbonRail::view_left(RIBBON_LEFT, "usdview.mara.ribbons")
        .default_open(PANE_TREE)
        .pane(
            PANE_SELECTION,
            ICON_SELECTION,
            "File & Selection",
            PaneAnchor::LeftRail(RailZone::Start),
            move |body| selection::render(body, &selection_snap, accent, &q_sel),
        )
        .pane(
            PANE_TREE,
            ICON_TREE,
            "Prim Tree (T)",
            PaneAnchor::LeftRail(RailZone::Start),
            move |body| tree::render(body, &tree_snap, accent, &q_tree),
        )
        .pane(
            PANE_INFO,
            ICON_INFO,
            "Stage Info & Diff (I)",
            PaneAnchor::LeftRail(RailZone::Start),
            move |body| info::render(body, &info_snap, accent, &q_info),
        )
        .pane(
            PANE_BIM,
            ICON_BIM,
            "BIM Properties & Classification",
            PaneAnchor::LeftRail(RailZone::Middle),
            move |body| bim::render(body, &bim_snap, accent, &q_bim),
        )
        .pane(
            PANE_VARIANTS,
            ICON_VARIANTS,
            "Variants & Clips",
            PaneAnchor::LeftRail(RailZone::Middle),
            move |body| variants::render(body, &variants_snap, accent, &q_var),
        )
        .pane(
            PANE_CAMERAS,
            ICON_CAMERAS,
            "Cameras & Bookmarks",
            PaneAnchor::LeftRail(RailZone::Middle),
            move |body| cameras::render(body, &cameras_snap, accent, &q_cam),
        )
        .pane(
            PANE_MATERIALS,
            ICON_MATERIALS,
            "Materials",
            PaneAnchor::LeftRail(RailZone::Middle),
            move |body| materials::render(body, &materials_snap, accent, &q_mat),
        )
        .pane(
            PANE_OVERLAYS,
            ICON_OVERLAYS,
            "Overlays & Settings (O)",
            PaneAnchor::LeftRail(RailZone::End),
            move |body| overlays::render(body, &overlays_snap, accent, &q_ovl),
        )
        .pane(
            PANE_TIMELINE,
            ICON_TIMELINE,
            "Timeline",
            PaneAnchor::LeftRail(RailZone::End),
            move |body| timeline::render_timeline(body, &timeline_snap, accent, &q_tl),
        )
        .pane(
            PANE_KEYS,
            ICON_KEYS,
            "Controls (?)",
            PaneAnchor::LeftRail(RailZone::End),
            |body| timeline::render_keys(body),
        )
        .pane(
            PANE_LOG,
            ICON_LOG,
            "Loader Log",
            PaneAnchor::LeftRail(RailZone::End),
            move |body| timeline::render_log(body, &log_snap, accent, &q_log),
        )
        .action_in(
            RibbonCluster::Middle,
            ACTION_OPEN,
            ICON_OPEN,
            "Open USD…",
            ribbon_action(ACTION_OPEN),
        )
        .action_in(
            RibbonCluster::Middle,
            ACTION_SAVE,
            ICON_SAVE,
            "Save Stage",
            ribbon_action(ACTION_SAVE),
        )
        .action_in(
            RibbonCluster::Middle,
            ACTION_UNDO,
            ICON_UNDO,
            "Undo (Ctrl+Z)",
            ribbon_action(ACTION_UNDO),
        )
        .action_in(
            RibbonCluster::Middle,
            ACTION_REDO,
            ICON_REDO,
            "Redo (Ctrl+Y)",
            ribbon_action(ACTION_REDO),
        )
        .action_in(
            RibbonCluster::End,
            ACTION_FRAME,
            ICON_FRAME,
            "Frame Visible Scene",
            ribbon_action(ACTION_FRAME),
        )
        .action_in(
            RibbonCluster::End,
            ACTION_PLAY_PHYSICS,
            ICON_PHYSICS,
            "Toggle Physics",
            ribbon_action(ACTION_PLAY_PHYSICS),
        )
        .action_in(
            RibbonCluster::End,
            ACTION_RELOAD,
            ICON_RELOAD,
            "Reload Stage (R)",
            ribbon_action(ACTION_RELOAD),
        )
}

fn handle_rail_actions(
    clicks: Vec<RibbonSlotClick>,
    world: &mut World,
    ui_state: &mut MaraHostUiState,
) {
    for click in clicks {
        if click.action == ribbon_action(ACTION_OPEN) {
            spawn_open_usd_dialog(ui_state);
        } else if click.action == ribbon_action(ACTION_SAVE) {
            world
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SaveStage);
        } else if click.action == ribbon_action(ACTION_UNDO) {
            world
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::UndoEditor);
        } else if click.action == ribbon_action(ACTION_REDO) {
            world
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::RedoEditor);
        } else if click.action == ribbon_action(ACTION_RELOAD) {
            world
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::ReloadSession);
        } else if click.action == ribbon_action(ACTION_FRAME) {
            apply_panel_action(PanelAction::FrameVisibleScene, world, ui_state);
        } else if click.action == ribbon_action(ACTION_PLAY_PHYSICS) {
            let running = world
                .resource::<ViewportReadModelState>()
                .snapshot()
                .is_some_and(|s| s.physics_running);
            world
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SetPhysicsRunning { running: !running });
        }
    }
}

pub(super) fn apply_panel_action(
    action: PanelAction,
    world: &mut World,
    ui_state: &mut MaraHostUiState,
) {
    if crate::viewport::input::viewer_interaction_locked(world)
        && matches!(
            &action,
            PanelAction::SaveStageAsDialog
                | PanelAction::CaptureDiffBaseline
                | PanelAction::ClearDiffBaseline
                | PanelAction::SaveBookmark
                | PanelAction::JumpBookmark(_)
                | PanelAction::DeleteBookmark(_)
                | PanelAction::FrameVisibleScene
        )
    {
        return;
    }
    match action {
        PanelAction::Command(command) => match command {
            ViewportCommand::RequestBimProperties => {
                let can_request = world
                    .resource::<ViewportReadModelState>()
                    .snapshot()
                    .is_some_and(|s| s.stage.loaded && !s.selection.targets.is_empty());
                if can_request {
                    let request_id = world
                        .resource_mut::<ViewportCommandInbox>()
                        .send(ViewportCommand::RequestBimProperties);
                    let _ = world
                        .resource_mut::<ViewportReadModelState>()
                        .begin_bim_properties_request(request_id);
                } else {
                    let _ = world
                        .resource_mut::<ViewportReadModelState>()
                        .begin_bim_properties_request(String::new());
                }
            }
            ViewportCommand::RequestBimClassificationFieldCatalogue { .. } => {
                let stage_loaded = world
                    .resource::<ViewportReadModelState>()
                    .snapshot()
                    .is_some_and(|s| s.stage.loaded);
                if stage_loaded {
                    let request_id = world.resource_mut::<ViewportCommandInbox>().send(command);
                    let _ = world
                        .resource_mut::<ViewportReadModelState>()
                        .begin_bim_catalogue_request(request_id);
                } else {
                    let _ = world
                        .resource_mut::<ViewportReadModelState>()
                        .begin_bim_catalogue_request(String::new());
                }
            }
            other => {
                world.resource_mut::<ViewportCommandInbox>().send(other);
            }
        },
        PanelAction::OpenUsdDialog => spawn_open_usd_dialog(ui_state),
        PanelAction::SaveStageAsDialog => spawn_save_as_dialog(ui_state),
        PanelAction::RevealInFilesystem => {
            let root = world
                .get_resource::<crate::viewport::stage::RequestedAsset>()
                .map(|r| r.root.clone())
                .unwrap_or_else(|| PathBuf::from("."));
            let name = world
                .get_resource::<StageInfo>()
                .map(|s| s.path.clone())
                .unwrap_or_default();
            let full = root.join(&name);
            let target = full.parent().unwrap_or(&root).to_path_buf();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer").arg(&target).spawn();
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&target).spawn();
            #[cfg(all(unix, not(target_os = "macos")))]
            let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
            let _ = LoadRequest::default();
        }
        PanelAction::ClearErrorBanner => {
            world
                .resource_mut::<ViewportReadModelState>()
                .clear_last_error();
        }
        PanelAction::CaptureDiffBaseline => {
            world
                .resource_mut::<SemanticDiffState>()
                .capture_baseline();
        }
        PanelAction::ClearDiffBaseline => {
            world
                .resource_mut::<SemanticDiffState>()
                .clear_baseline();
        }
        PanelAction::ClearLog => {
            if let Some(log) = world.get_resource::<LoaderLog>()
                && let Ok(mut buf) = log.buffer.lock()
            {
                buf.clear();
            }
        }
        PanelAction::SaveBookmark => {
            let mut cam_q = world.query::<&ArcballCamera>();
            if let Ok(cam) = cam_q.single(world).cloned() {
                let mut bookmarks = world.resource_mut::<CameraBookmarks>();
                let seq = bookmarks.next_seq + 1;
                bookmarks.next_seq = seq;
                bookmarks.items.push(CameraBookmark {
                    name: format!("View {seq}"),
                    focus: cam.focus,
                    distance: cam.distance,
                    yaw: cam.yaw,
                    elevation: cam.elevation,
                });
            }
        }
        PanelAction::JumpBookmark(idx) => {
            let bm = world
                .resource::<CameraBookmarks>()
                .items
                .get(idx)
                .cloned();
            let mut cam_q = world.query::<&ArcballCamera>();
            if let (Some(bm), Ok(cam)) = (bm, cam_q.single(world).cloned()) {
                *world.resource_mut::<CameraMount>() = CameraMount::Arcball;
                let mut fly = world.resource_mut::<FlyTo>();
                fly.start_focus = cam.focus;
                fly.start_distance = cam.distance;
                fly.start_yaw = Some(cam.yaw);
                fly.start_elevation = Some(cam.elevation);
                fly.target_focus = bm.focus;
                fly.target_distance = bm.distance;
                fly.target_yaw = Some(bm.yaw);
                fly.target_elevation = Some(bm.elevation);
                fly.duration = 0.5;
                fly.remaining = 0.5;
            }
        }
        PanelAction::DeleteBookmark(idx) => {
            let mut bookmarks = world.resource_mut::<CameraBookmarks>();
            if idx < bookmarks.items.len() {
                bookmarks.items.remove(idx);
            }
        }
        PanelAction::FrameVisibleScene => {
            let extent = world
                .get_resource::<crate::viewport::scene::visualization::SceneExtent>()
                .cloned();
            if let Some(extent) = extent {
                *world.resource_mut::<CameraMount>() = CameraMount::Arcball;
                let diag = extent.diag().max(2.0);
                let focus = extent.centre();
                let distance = diag * 1.1;
                let mut q = world.query::<(
                    &mut Transform,
                    &mut ArcballCamera,
                    Option<&mut bevy::camera_controller::pan_orbit_camera::controller::component::PanOrbitCamera>,
                )>();
                if let Ok((mut tr, mut cam, pan_orbit)) = q.single_mut(world) {
                    cam.focus = focus;
                    cam.distance = distance;
                    cam.zoom_target = f64::from(distance);
                    crate::viewport::camera::apply_rig(&cam, &mut tr);
                    if let Some(mut pan_orbit) = pan_orbit {
                        sync_pan_orbit_from_arcball(&cam, &mut pan_orbit);
                    }
                }
            }
        }
        PanelAction::TreeSearch(query) => {
            let trimmed = query.trim().to_owned();
            ui_state.tree.filter = query;
            if trimmed.is_empty() {
                world.resource_mut::<ViewportReadModelState>().clear_search();
            } else {
                let request_id = world
                    .resource_mut::<ViewportCommandInbox>()
                    .send(ViewportCommand::SearchScene {
                        query: trimmed.clone(),
                        offset: 0,
                        limit: viewport_protocol::DEFAULT_SCENE_SEARCH_PAGE_SIZE,
                    });
                world
                    .resource_mut::<ViewportReadModelState>()
                    .begin_search(request_id, trimmed);
            }
        }
        PanelAction::TreeLoadMoreSearch => {
            let next = world
                .resource::<ViewportReadModelState>()
                .next_search_page();
            if let Some((query, offset)) = next {
                world
                    .resource_mut::<ViewportCommandInbox>()
                    .send(ViewportCommand::SearchScene {
                        query,
                        offset,
                        limit: viewport_protocol::DEFAULT_SCENE_SEARCH_PAGE_SIZE,
                    });
            }
        }
        PanelAction::TreeExpand(anchor, open) => {
            ui_state.tree.expanded.insert(anchor.clone(), open);
            if open {
                let page_requests = {
                    let mut rm = world.resource_mut::<ViewportReadModelState>();
                    rm.request_scene_children(anchor);
                    rm.take_scene_page_requests()
                };
                let mut inbox = world.resource_mut::<ViewportCommandInbox>();
                for req in page_requests {
                    inbox.send(ViewportCommand::RequestSceneChildren {
                        parent: req.parent,
                        page: req.page,
                        page_size: req.page_size,
                    });
                }
            }
        }
        PanelAction::TreeExpandAllLoaded(anchors) => {
            let mut rm = world.resource_mut::<ViewportReadModelState>();
            for anchor in anchors {
                ui_state.tree.expanded.insert(anchor.clone(), true);
                rm.request_scene_children(anchor);
            }
            let page_requests = rm.take_scene_page_requests();
            drop(rm);
            let mut inbox = world.resource_mut::<ViewportCommandInbox>();
            for req in page_requests {
                inbox.send(ViewportCommand::RequestSceneChildren {
                    parent: req.parent,
                    page: req.page,
                    page_size: req.page_size,
                });
            }
        }
        PanelAction::SetCataloguePage(page) => {
            ui_state.pages.catalogue_page = page;
        }
        PanelAction::SetPropertyPage { group_id, page } => {
            ui_state.pages.property_pages.insert(group_id, page);
        }
        PanelAction::SetMaterialPage(page) => {
            ui_state.pages.material_page = page;
        }
    }
}

fn spawn_open_usd_dialog(ui_state: &mut MaraHostUiState) {
    if ui_state.pending_dialog.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    ui_state.pending_dialog = Some(PendingFileDialog::OpenUsd(rx));
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
            .pick_file();
        let _ = tx.send(picked);
    });
}

fn spawn_save_as_dialog(ui_state: &mut MaraHostUiState) {
    if ui_state.pending_dialog.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    ui_state.pending_dialog = Some(PendingFileDialog::SaveStageAs(rx));
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .add_filter("USD stages", &["usda", "usdc", "usd"])
            .save_file();
        let _ = tx.send(picked);
    });
}
