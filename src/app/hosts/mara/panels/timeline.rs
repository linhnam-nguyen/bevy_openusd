use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32 as MaraColor32, Id as MaraId};
use viewport_protocol::{TimelineReadModel, ViewportCommand};

use super::{ActionQueue, PanelAction, push_action};
use crate::api::ViewportReadModelState;
use crate::diagnostics::log_capture::{LoaderLog, LogLine};
use crate::viewport::stage::StageInfo;

#[derive(Clone)]
pub(super) struct TimelineSnapshot {
    timeline: Option<TimelineReadModel>,
    animated_prim_count: usize,
}

#[derive(Clone)]
pub(super) struct LogSnapshot {
    lines: Vec<LogLine>,
}

pub(super) fn snapshot(world: &World) -> TimelineSnapshot {
    let timeline = world
        .resource::<ViewportReadModelState>()
        .snapshot()
        .map(|s| s.timeline.clone());
    let animated_prim_count = world.resource::<StageInfo>().animated_prim_count;
    TimelineSnapshot {
        timeline,
        animated_prim_count,
    }
}

pub(super) fn log_snapshot(world: &World) -> LogSnapshot {
    let lines = world
        .get_resource::<LoaderLog>()
        .and_then(|log| log.buffer.lock().ok().map(|b| b.iter().cloned().collect()))
        .unwrap_or_default();
    LogSnapshot { lines }
}

pub(super) fn render_timeline(
    body: &mut PaneBody,
    snap: &TimelineSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let Some(tl) = &snap.timeline else {
        body.add_normal(
            "usd.timeline.wait",
            "Timeline",
            "clock",
            vec![
                Pod::new(MaraId::new(("mara.tl", "wait")))
                    .with_readout("status", "Waiting for snapshot…"),
            ],
        );
        return;
    };

    let duration_seconds = (tl.end_time_code - tl.start_time_code).max(0.0)
        / tl.time_codes_per_second.max(f64::MIN_POSITIVE);
    let mut seconds = tl.seconds;
    let playing = tl.playing;
    let dur = duration_seconds.max(1e-3);
    let q = queue.clone();

    body.add_normal(
        "usd.timeline.playback",
        "Playback",
        "clock",
        vec![
            Pod::new(MaraId::new(("mara.tl", "meta"))).with_readout(
                "animated",
                format!(
                    "{} prim(s) · {:.1} fps · {:.1}s",
                    snap.animated_prim_count, tl.time_codes_per_second, duration_seconds
                ),
            ),
            Pod::new(MaraId::new(("mara.tl", "controls"))).with_custom_units(4, move |ui| {
                ui.horizontal(|ui| {
                    let play_label = if playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() {
                        push_action(
                            &q,
                            PanelAction::Command(ViewportCommand::SetPlayback {
                                playing: !playing,
                            }),
                        );
                    }
                    if ui.button("Rewind").clicked() {
                        push_action(
                            &q,
                            PanelAction::Command(ViewportCommand::Seek { seconds: 0.0 }),
                        );
                    }
                });
                if ui
                    .slider("Seconds", &mut seconds, 0.0..=dur, 3, " s")
                    .changed()
                {
                    push_action(
                        &q,
                        PanelAction::Command(ViewportCommand::Seek { seconds }),
                    );
                }
            }),
            Pod::new(MaraId::new(("mara.tl", "tc"))).with_readout(
                "timeCode",
                format!(
                    "{:.3}",
                    tl.start_time_code + tl.seconds * tl.time_codes_per_second
                ),
            ),
            Pod::new(MaraId::new(("mara.tl", "range"))).with_readout(
                "range",
                format!("{:.2} … {:.2}", tl.start_time_code, tl.end_time_code),
            ),
        ],
    );
    let _ = accent;
}

pub(super) fn render_keys(body: &mut PaneBody) {
    body.add_normal(
        "usd.keys.camera",
        "Camera",
        "keyboard",
        vec![
            Pod::new(MaraId::new(("mara.keys", "cam"))).with_keybindings([
                ("L+R drag", "Orbit"),
                ("Middle / Shift drag", "Pan"),
                ("Scroll", "Focus-centered zoom"),
            ]),
        ],
    );
    body.add_normal(
        "usd.keys.panels",
        "Panels & Overlays",
        "keyboard",
        vec![
            Pod::new(MaraId::new(("mara.keys", "shortcuts"))).with_keybindings([
                ("T", "Toggle Prim tree"),
                ("I", "Toggle Stage info"),
                ("O", "Toggle Overlays"),
                ("?", "Toggle Controls"),
                ("G / X / P", "Grid / Axes / Markers"),
                ("B / Y / C", "Skeleton / Physics / Colliders"),
                ("R", "Reload stage"),
                ("Ctrl+Z / Ctrl+Y", "Undo / Redo"),
            ]),
        ],
    );
}

pub(super) fn render_log(
    body: &mut PaneBody,
    snap: &LogSnapshot,
    accent: MaraColor32,
    queue: &ActionQueue,
) {
    let q = queue.clone();
    let lines: Vec<String> = snap
        .lines
        .iter()
        .rev()
        .take(80)
        .map(|line| {
            let short = line.target.rsplit("::").next().unwrap_or(&line.target);
            format!("[{:?}] {}: {}", line.level, short, line.message)
        })
        .collect();
    let count = snap.lines.len();
    body.add_normal(
        "usd.log",
        "Loader Log",
        "history",
        vec![
            Pod::new(MaraId::new(("mara.log", "count")))
                .with_readout("entries", format!("{count} (cap 500)")),
            Pod::new(MaraId::new(("mara.log", "clear"))).with_custom_units(1, move |ui| {
                if ui.button("Clear log").clicked() {
                    push_action(&q, PanelAction::ClearLog);
                }
            }),
            Pod::new(MaraId::new(("mara.log", "lines")))
                .fill()
                .with_custom_units(10, move |ui| {
                    if lines.is_empty() {
                        ui.label("(no events yet)");
                    } else {
                        for line in &lines {
                            ui.label(line);
                        }
                    }
                }),
        ],
    );
    let _ = accent;
}
