use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{CameraSource, StandardView, ViewportCommand};

use super::{ActionQueue, PanelAction, push_action};
use crate::viewport::camera::{CameraBookmark, CameraBookmarks, CameraMount};
use crate::viewport::stage::{StageCameraInfo, StageCameraProjection, StageInfo};

#[derive(Clone)]
pub(super) struct CamerasSnapshot {
    bookmarks: Vec<CameraBookmark>,
    mount: CameraMount,
    authored_cameras: Vec<StageCameraInfo>,
}

pub(super) fn snapshot(world: &World) -> CamerasSnapshot {
    let bookmarks = world.resource::<CameraBookmarks>().items.clone();
    let mount = world.resource::<CameraMount>().clone();
    let authored_cameras = world.resource::<StageInfo>().cameras.clone();
    CamerasSnapshot {
        bookmarks,
        mount,
        authored_cameras,
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &CamerasSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let q_views = queue.clone();
    body.add_normal(
        "usd.cam.views",
        "Standard Views",
        "cube",
        vec![
            Pod::new(MaraId::new(("mara.cam", "std_views"))).with_custom_units(3, move |ui| {
                ui.horizontal(|ui| {
                    for (label, view) in [
                        ("Front", StandardView::Front),
                        ("Back", StandardView::Back),
                        ("Left", StandardView::Left),
                    ] {
                        if ui.button(label).clicked() {
                            push_action(
                                &q_views,
                                PanelAction::Command(ViewportCommand::SetStandardView { view }),
                            );
                        }
                    }
                });
                ui.horizontal(|ui| {
                    for (label, view) in [
                        ("Right", StandardView::Right),
                        ("Top", StandardView::Top),
                        ("Bottom", StandardView::Bottom),
                    ] {
                        if ui.button(label).clicked() {
                            push_action(
                                &q_views,
                                PanelAction::Command(ViewportCommand::SetStandardView { view }),
                            );
                        }
                    }
                });
            }),
        ],
    );

    let q_save = queue.clone();
    let mut bm_pods = vec![
        Pod::new(MaraId::new(("mara.cam", "bm_save"))).with_custom_units(1, move |ui| {
            if ui.button("Save current view").clicked() {
                push_action(&q_save, PanelAction::SaveBookmark);
            }
        }),
    ];
    if snap.bookmarks.is_empty() {
        bm_pods.push(
            Pod::new(MaraId::new(("mara.cam", "bm_none")))
                .with_readout("bookmarks", "(none yet)"),
        );
    } else {
        for (idx, bm) in snap.bookmarks.iter().enumerate() {
            let q_bm = queue.clone();
            let name = bm.name.clone();
            let dist = format!("d {:.1}", bm.distance);
            bm_pods.push(
                Pod::new(MaraId::new(("mara.cam.bm", idx))).with_custom_units(1, move |ui| {
                    let resp =
                        ui.hybrid_select_row(("mara.cam.bm", idx), &name, Some(&dist), false, false);
                    if resp.body.clicked {
                        push_action(&q_bm, PanelAction::JumpBookmark(idx));
                    }
                    if resp.radio.clicked {
                        push_action(&q_bm, PanelAction::DeleteBookmark(idx));
                    }
                }),
            );
        }
    }
    body.add_normal("usd.cam.bookmarks", "Bookmarks", "cube", bm_pods);

    let arcball_active = matches!(snap.mount, CameraMount::Arcball);
    let q_free = queue.clone();
    let mut src_pods = vec![
        Pod::new(MaraId::new(("mara.cam", "free"))).with_custom_units(1, move |ui| {
            let resp = ui.hybrid_select_row(
                ("mara.cam", "free"),
                "PanOrbit (free)",
                None,
                arcball_active,
                arcball_active,
            );
            if resp.body.clicked || resp.radio.clicked {
                push_action(
                    &q_free,
                    PanelAction::Command(ViewportCommand::SetCameraSource {
                        source: CameraSource::Arcball,
                    }),
                );
            }
        }),
    ];
    for (idx, cam) in snap.authored_cameras.iter().enumerate() {
        let mounted = matches!(
            &snap.mount,
            CameraMount::Mounted { prim_path } if prim_path == &cam.path
        );
        let name = cam.path.rsplit('/').next().unwrap_or(&cam.path).to_owned();
        let focal = cam.data.focal_length_mm.unwrap_or(50.0);
        let proj = match cam.data.projection {
            Some(StageCameraProjection::Orthographic) => "ortho",
            _ => "persp",
        };
        let trailing = format!("{focal:.0}mm · {proj}");
        let path = cam.path.clone();
        let q_cam = queue.clone();
        src_pods.push(
            Pod::new(MaraId::new(("mara.cam.usd", idx))).with_custom_units(1, move |ui| {
                let resp = ui.hybrid_select_row(
                    ("mara.cam.usd", idx),
                    &name,
                    Some(&trailing),
                    mounted,
                    mounted,
                );
                if resp.body.clicked || resp.radio.clicked {
                    push_action(
                        &q_cam,
                        PanelAction::Command(ViewportCommand::SetCameraSource {
                            source: CameraSource::Authored {
                                prim_path: path.clone(),
                            },
                        }),
                    );
                }
            }),
        );
    }
    let _ = accent;
    body.add_normal("usd.cam.sources", "Authored Cameras", "cube", src_pods);
}
