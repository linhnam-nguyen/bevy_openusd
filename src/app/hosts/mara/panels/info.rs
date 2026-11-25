use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use usd_bevy::{UsdPrimRef, UsdProcedural, UsdSpatialAudio};
use usd_diff::DiffSummary;
use viewport_protocol::ViewportCommand;

use super::{ActionQueue, PanelAction, push_action};
use crate::semantic::viewport::SemanticDiffState;
use crate::viewport::stage::StageInfo;

#[derive(Clone)]
pub(super) struct InfoSnapshot {
    path: String,
    default_prim: Option<String>,
    layer_count: usize,
    prim_count: usize,
    mesh_count: usize,
    variant_count: usize,
    lights: [usize; 4],
    instance_prims: usize,
    instance_reuses: usize,
    animated_prims: usize,
    skel: [usize; 3],
    render: [usize; 3],
    resolution: Option<[i32; 2]>,
    physics: [usize; 3],
    custom_attrs: usize,
    custom_layers: usize,
    subdiv_prims: usize,
    light_linked: usize,
    clips: usize,
    spatial_audio: usize,
    procedural: usize,
    has_baseline: bool,
    has_working: bool,
    diff_summary: Option<DiffSummary>,
}

pub(super) fn snapshot(world: &World) -> InfoSnapshot {
    let info = world.resource::<StageInfo>();
    let diff = world.resource::<SemanticDiffState>();
    let prim_count = world
        .iter_entities()
        .filter(|e| e.contains::<UsdPrimRef>())
        .count();
    let mesh_count = world
        .iter_entities()
        .filter(|e| e.contains::<UsdPrimRef>() && e.contains::<Mesh3d>())
        .count();
    let spatial_audio = world
        .iter_entities()
        .filter(|e| e.contains::<UsdSpatialAudio>())
        .count();
    let procedural = world
        .iter_entities()
        .filter(|e| e.contains::<UsdProcedural>())
        .count();

    InfoSnapshot {
        path: info.path.clone(),
        default_prim: info.default_prim.clone(),
        layer_count: info.layer_count,
        prim_count,
        mesh_count,
        variant_count: info.variant_count,
        lights: [
            info.lights_directional,
            info.lights_point,
            info.lights_spot,
            info.lights_dome,
        ],
        instance_prims: info.instance_prim_count,
        instance_reuses: info.instance_prototype_reuses,
        animated_prims: info.animated_prim_count,
        skel: [info.skeleton_count, info.skel_root_count, info.skel_binding_count],
        render: [
            info.render_settings_count,
            info.render_product_count,
            info.render_var_count,
        ],
        resolution: info.render_primary_resolution,
        physics: [
            info.physics_scene_count,
            info.rigid_body_count,
            info.joint_count,
        ],
        custom_attrs: info.custom_attr_prim_count,
        custom_layers: info.custom_layer_data_entries,
        subdiv_prims: info.subdivision_prim_count,
        light_linked: info.light_linked_count,
        clips: info.clip_prim_count,
        spatial_audio,
        procedural,
        has_baseline: diff.has_baseline(),
        has_working: diff.has_working_snapshot(),
        diff_summary: diff.summary(),
    }
}

pub(super) fn render(
    body: &mut PaneBody,
    snap: &InfoSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let q_reload = queue.clone();
    body.add_normal(
        "usd.info.stage",
        "Stage",
        "list",
        vec![
            Pod::new(MaraId::new(("mara.info", "file"))).with_readout("file", &snap.path),
            Pod::new(MaraId::new(("mara.info", "defaultPrim"))).with_readout(
                "defaultPrim",
                snap.default_prim.as_deref().unwrap_or("—"),
            ),
            Pod::new(MaraId::new(("mara.info", "counts"))).with_badge_row(
                "counts",
                vec![
                    format!("{} layers", snap.layer_count),
                    format!("{} prims", snap.prim_count),
                    format!("{} meshes", snap.mesh_count),
                    format!("{} variants", snap.variant_count),
                ],
                accent,
            ),
            Pod::new(MaraId::new(("mara.info", "reload"))).with_custom_units(1, move |ui| {
                if ui.button("Reload stage (R)").clicked() {
                    push_action(
                        &q_reload,
                        PanelAction::Command(ViewportCommand::ReloadSession),
                    );
                }
            }),
        ],
    );

    let mut diff_pods = Vec::new();
    if !snap.has_baseline {
        if snap.has_working {
            let q_cap = queue.clone();
            diff_pods.push(
                Pod::new(MaraId::new(("mara.info", "diff_cap"))).with_custom_units(1, move |ui| {
                    if ui.button("Capture current as baseline").clicked() {
                        push_action(&q_cap, PanelAction::CaptureDiffBaseline);
                    }
                }),
            );
        } else {
            diff_pods.push(
                Pod::new(MaraId::new(("mara.info", "diff_wait")))
                    .with_readout("status", "Waiting for semantic snapshot"),
            );
        }
    } else if let Some(sum) = &snap.diff_summary {
        diff_pods.push(Pod::new(MaraId::new(("mara.info", "diff_counts"))).with_badge_row(
            "diff",
            vec![
                format!("+{}", sum.added),
                format!("-{}", sum.removed),
                format!("~{}", sum.changed),
                format!("={} ", sum.unchanged),
            ],
            accent,
        ));
        diff_pods.push(Pod::new(MaraId::new(("mara.info", "diff_flags"))).with_badge_row(
            "flags",
            vec![
                format!("{} xform", sum.transform),
                format!("{} geom", sum.geometry),
                format!("{} meta", sum.metadata),
                format!("{} path", sum.path),
            ],
            accent,
        ));
        let q_clr = queue.clone();
        diff_pods.push(
            Pod::new(MaraId::new(("mara.info", "diff_clr"))).with_custom_units(1, move |ui| {
                if ui.button("Clear manual baseline").clicked() {
                    push_action(&q_clr, PanelAction::ClearDiffBaseline);
                }
            }),
        );
    }
    body.add_normal("usd.info.diff", "Working Diff", "branch", diff_pods);

    let mut detail_pods = vec![
        Pod::new(MaraId::new(("mara.info", "lights"))).with_badge_row(
            "lights",
            vec![
                format!("{} dir", snap.lights[0]),
                format!("{} pt", snap.lights[1]),
                format!("{} spot", snap.lights[2]),
                format!("{} dome", snap.lights[3]),
            ],
            accent,
        ),
        Pod::new(MaraId::new(("mara.info", "inst"))).with_badge_row(
            "instances",
            vec![
                format!("{} prim", snap.instance_prims),
                format!("{} reuse", snap.instance_reuses),
                format!("{} anim", snap.animated_prims),
            ],
            accent,
        ),
        Pod::new(MaraId::new(("mara.info", "skel"))).with_badge_row(
            "skel",
            vec![
                format!("{} skel", snap.skel[0]),
                format!("{} root", snap.skel[1]),
                format!("{} bind", snap.skel[2]),
            ],
            accent,
        ),
        Pod::new(MaraId::new(("mara.info", "render"))).with_badge_row(
            "render",
            vec![
                format!("{} settings", snap.render[0]),
                format!("{} product", snap.render[1]),
                format!("{} var", snap.render[2]),
            ],
            accent,
        ),
        Pod::new(MaraId::new(("mara.info", "physics"))).with_badge_row(
            "physics",
            vec![
                format!("{} scene", snap.physics[0]),
                format!("{} rigid", snap.physics[1]),
                format!("{} joint", snap.physics[2]),
            ],
            accent,
        ),
        Pod::new(MaraId::new(("mara.info", "author"))).with_readout(
            "authoring",
            format!(
                "{} custom · {} layers · {} subdiv · {} clips · {} audio · {} proc · {} light-link",
                snap.custom_attrs,
                snap.custom_layers,
                snap.subdiv_prims,
                snap.clips,
                snap.spatial_audio,
                snap.procedural,
                snap.light_linked,
            ),
        ),
    ];
    if let Some([w, h]) = snap.resolution {
        detail_pods.push(
            Pod::new(MaraId::new(("mara.info", "res")))
                .with_readout("resolution", format!("{w} × {h}")),
        );
    }
    body.add_normal("usd.info.details", "Scene Summary", "cube", detail_pods);
}
