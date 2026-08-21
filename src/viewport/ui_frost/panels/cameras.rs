use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use viewport_protocol::ViewportCommand;

use crate::viewport::api::ViewportCommandInbox;
use crate::viewport::camera::{ArcballCamera, CameraBookmark, CameraBookmarks, CameraMount, FlyTo};
use crate::viewport::session::{StageCameraProjection, StageInfo};
use crate::viewport::ui_frost::constants::{PANEL_H, PANEL_W, RIB_CAMERAS, RIBBON_ITEMS, RIBBONS};
use crate::viewport::ui_frost::plugin::is_panel_open;

#[allow(clippy::too_many_arguments)]
/// Lists authored cameras and manages mounting, bookmarks, and camera navigation.
pub fn draw_cameras_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    stage_info: Res<StageInfo>,
    mut camera_mount: ResMut<CameraMount>,
    mut bookmarks: ResMut<CameraBookmarks>,
    mut fly: ResMut<FlyTo>,
    cameras: Query<&ArcballCamera>,
    mut viewport_commands: ResMut<ViewportCommandInbox>,
) {
    if !is_panel_open(&open, RIB_CAMERAS) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_CAMERAS,
        "Cameras",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("cameras_bookmarks", "Bookmarks", true, |ui| {
                if wide_button(ui, "💾  Save current view", accent_col).clicked()
                    && let Ok(cam) = cameras.single()
                {
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
                if bookmarks.items.is_empty() {
                    sub_caption(ui, "(no bookmarks yet)");
                } else {
                    let mut to_delete: Option<usize> = None;
                    let mut to_jump: Option<usize> = None;
                    for (idx, bm) in bookmarks.items.iter().enumerate() {
                        let r = hybrid_select_row(
                            ui,
                            ("bookmark", idx),
                            &bm.name,
                            Some(&format!("d {:.1}", bm.distance)),
                            false,
                            false,
                            accent_col,
                        );
                        if r.body.clicked() {
                            to_jump = Some(idx);
                        }
                        if r.radio.clicked() {
                            to_delete = Some(idx);
                        }
                    }
                    if let Some(idx) = to_jump
                        && let (Ok(cam), Some(bm)) = (cameras.single(), bookmarks.items.get(idx))
                    {
                        *camera_mount = CameraMount::Arcball;
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
                    if let Some(idx) = to_delete {
                        bookmarks.items.remove(idx);
                    }
                    sub_caption(ui, "Click row to jump · click radio to delete");
                }
            });

            pane.section("cameras_all", "Cameras", true, |ui| {
                let asset = Some(&*stage_info);
                let Some(asset) = asset else {
                    sub_caption(ui, "(no stage loaded yet)");
                    return;
                };
                sub_caption(ui, &format!("{} authored cameras", asset.cameras.len()));
                ui.add_space(style::space::BLOCK);

                let arcball_active = matches!(*camera_mount, CameraMount::Arcball);
                let r = hybrid_select_row(
                    ui,
                    "arcball_mount",
                    "🎮  Arcball (free)",
                    None,
                    arcball_active,
                    arcball_active,
                    accent_col,
                );
                if r.body.clicked() || r.radio.clicked() {
                    viewport_commands.send(ViewportCommand::SetCameraSource {
                        source: viewport_protocol::CameraSource::Arcball,
                    });
                }

                row_separator(ui);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cam in &asset.cameras {
                        let mounted = matches!(
                            &*camera_mount,
                            CameraMount::Mounted { prim_path } if prim_path == &cam.path
                        );
                        let name = cam.path.rsplit('/').next().unwrap_or(&cam.path);
                        let focal = cam.data.focal_length_mm.unwrap_or(50.0);
                        let proj = match cam.data.projection {
                            Some(StageCameraProjection::Orthographic) => "ortho",
                            _ => "persp",
                        };
                        let label = format!("📷  {name}");
                        let trailing = format!("{focal:.0}mm · {proj}");
                        let r = hybrid_select_row(
                            ui,
                            cam.path.as_str(),
                            &label,
                            Some(&trailing),
                            mounted,
                            mounted,
                            accent_col,
                        );
                        if r.body.clicked() || r.radio.clicked() {
                            viewport_commands.send(ViewportCommand::SetCameraSource {
                                source: viewport_protocol::CameraSource::Authored {
                                    prim_path: cam.path.clone(),
                                },
                            });
                        }
                    }
                });
            });
        },
    );
}
