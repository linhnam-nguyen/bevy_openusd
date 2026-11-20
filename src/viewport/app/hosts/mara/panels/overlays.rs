use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{OverlayKind, PresentationReadModel, RenderMode, ViewportCommand};

use super::{ActionQueue, PanelAction, push_action};
use crate::viewport::api::ViewportReadModelState;
use crate::viewport::section_box::SectionBoxState;

#[derive(Clone)]
pub(super) struct OverlaysSnapshot {
    presentation: Option<PresentationReadModel>,
    section_box_enabled: bool,
}

pub(super) fn snapshot(world: &World) -> OverlaysSnapshot {
    let presentation = world
        .resource::<ViewportReadModelState>()
        .snapshot()
        .map(|s| s.presentation.clone());
    let section_box_enabled = world
        .get_resource::<SectionBoxState>()
        .is_some_and(|s| s.enabled);
    OverlaysSnapshot {
        presentation,
        section_box_enabled,
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &OverlaysSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let Some(pres) = &snap.presentation else {
        body.add_normal(
            "usd.overlays.loading",
            "Overlays",
            "eye",
            vec![
                Pod::new(MaraId::new(("mara.ovl", "wait")))
                    .with_readout("status", "Waiting for snapshot…"),
            ],
        );
        return;
    };

    let overlay_items = [
        ("Ground grid (G)", OverlayKind::GroundGrid, pres.ground_grid),
        ("World axes (X)", OverlayKind::WorldAxes, pres.world_axes),
        ("Prim markers (P)", OverlayKind::PrimMarkers, pres.prim_markers),
        ("Skeleton bones (B)", OverlayKind::Skeleton, pres.skeleton),
        ("Physics gizmos (Y)", OverlayKind::Physics, pres.physics),
        ("Collider wireframes (C)", OverlayKind::Colliders, pres.colliders),
        ("Wireframe", OverlayKind::Wireframe, pres.wireframe),
    ];

    let q_ovl = queue.clone();
    let q_sb = queue.clone();
    let sb_enabled = snap.section_box_enabled;
    let mut bias = f64::from(pres.prim_marker_bias);

    body.add_normal(
        "usd.overlays.world",
        "World Overlays & Section Box",
        "eye",
        vec![
            Pod::new(MaraId::new(("mara.ovl", "toggles"))).with_custom_units(10, move |ui| {
                for (label, kind, current) in overlay_items {
                    let mut val = current;
                    if ui.toggle(label, &mut val).changed() {
                        push_action(
                            &q_ovl,
                            PanelAction::Command(ViewportCommand::SetOverlay {
                                overlay: kind,
                                enabled: val,
                            }),
                        );
                    }
                }
                let mut sb = sb_enabled;
                if ui.toggle("Section box", &mut sb).changed() {
                    push_action(
                        &q_sb,
                        PanelAction::Command(ViewportCommand::SetSectionBox { enabled: sb }),
                    );
                }
                if ui
                    .slider("Prim marker bias", &mut bias, 0.0..=5.0, 2, "×")
                    .changed()
                {
                    push_action(
                        &q_ovl,
                        PanelAction::Command(ViewportCommand::SetPrimMarkerBias {
                            bias: bias as f32,
                        }),
                    );
                }
            }),
        ],
    );

    let mut light_scale = f64::from(pres.light_intensity_scale);
    let mut curve_tuning = pres.curve_tuning;
    let current_mode = pres.renderer.render_mode;
    let mut renderer_cfg = pres.renderer.clone();
    let q_render = queue.clone();

    body.add_normal(
        "usd.overlays.render",
        "Rendering & Curves",
        "settings",
        vec![
            Pod::new(MaraId::new(("mara.ovl", "render_ctrl"))).with_custom_units(10, move |ui| {
                let modes = [
                    ("Shaded", RenderMode::Shaded),
                    ("Wireframe", RenderMode::Wireframe),
                    ("Uniform Color", RenderMode::UniformColor),
                    ("Solari Raytraced", RenderMode::RayTraced),
                ];
                let mut selected_mode = modes
                    .iter()
                    .position(|(_, m)| *m == current_mode)
                    .unwrap_or(0);
                let labels: Vec<&str> = modes.iter().map(|(l, _)| *l).collect();
                ui.readout("render mode", labels[selected_mode]);
                if ui
                    .dropdown("mara.ovl.render_mode", &mut selected_mode, &labels)
                    .changed()
                {
                    renderer_cfg.render_mode = modes[selected_mode].1;
                    push_action(
                        &q_render,
                        PanelAction::Command(ViewportCommand::SetRendererConfiguration {
                            configuration: renderer_cfg.clone(),
                        }),
                    );
                }
                if ui
                    .slider("Light intensity", &mut light_scale, 0.0..=5.0, 2, "×")
                    .changed()
                {
                    push_action(
                        &q_render,
                        PanelAction::Command(ViewportCommand::SetLightIntensity {
                            scale: light_scale as f32,
                        }),
                    );
                }
                let mut rad = f64::from(curve_tuning.default_radius);
                if ui
                    .slider("Curve radius", &mut rad, 0.001..=0.2, 3, " m")
                    .changed()
                {
                    curve_tuning.default_radius = rad as f32;
                    push_action(
                        &q_render,
                        PanelAction::Command(ViewportCommand::SetCurveTuning {
                            tuning: curve_tuning,
                        }),
                    );
                }
                let mut segs = f64::from(curve_tuning.ring_segments);
                if ui
                    .slider("Curve segments", &mut segs, 3.0..=24.0, 0, "")
                    .changed()
                {
                    curve_tuning.ring_segments = segs.round() as u32;
                    push_action(
                        &q_render,
                        PanelAction::Command(ViewportCommand::SetCurveTuning {
                            tuning: curve_tuning,
                        }),
                    );
                }
                let mut pt_scale = f64::from(curve_tuning.point_scale);
                if ui
                    .slider("Point scale", &mut pt_scale, 0.05..=4.0, 2, "×")
                    .changed()
                {
                    curve_tuning.point_scale = pt_scale as f32;
                    push_action(
                        &q_render,
                        PanelAction::Command(ViewportCommand::SetCurveTuning {
                            tuning: curve_tuning,
                        }),
                    );
                }
            }),
        ],
    );
    let _ = accent;
}
