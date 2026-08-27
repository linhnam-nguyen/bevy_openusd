use std::time::Instant;

use bevy::prelude::{App, World};
use viewport_protocol::SelectionReadModel;

use super::{SelectionColorOverrideState, SelectionOutlineState, set_selection};

const REPEATS: usize = 5;

fn presentation_pending(app: &World) -> bool {
    app.get_resource::<SelectionOutlineState>()
        .is_some_and(SelectionOutlineState::is_pending)
        || app
            .get_resource::<SelectionColorOverrideState>()
            .is_some_and(SelectionColorOverrideState::is_pending)
}

fn settle_presentation(app: &mut App) -> usize {
    let mut frames = 0;
    while presentation_pending(app.world()) {
        assert!(frames < 64, "selection presentation did not settle");
        app.update();
        frames += 1;
    }
    frames
}

pub(super) fn settle_selection_presentation(app: &mut App) {
    const MAX_SETTLE_UPDATES: usize = 32;

    for _ in 0..MAX_SETTLE_UPDATES {
        if !presentation_pending(app.world()) {
            return;
        }
        app.update();
    }

    panic!("selection presentation did not settle within {MAX_SETTLE_UPDATES} updates");
}

pub(super) fn repeat_selection_updates(
    app: &mut App,
    value: &SelectionReadModel,
    mut update: impl FnMut(&mut App),
) -> (u128, u128, usize) {
    let mut samples = Vec::with_capacity(REPEATS);
    let mut max_settle_frames = 0;
    for _ in 0..REPEATS {
        set_selection(app, value.clone());
        let started = Instant::now();
        update(app);
        samples.push(started.elapsed().as_micros());
        max_settle_frames = max_settle_frames.max(settle_presentation(app));
        set_selection(app, SelectionReadModel::default());
        app.update();
        max_settle_frames = max_settle_frames.max(settle_presentation(app));
    }
    set_selection(app, value.clone());
    app.update();
    max_settle_frames = max_settle_frames.max(settle_presentation(app));
    let maximum = samples.iter().copied().max().unwrap_or_default();
    samples.sort_unstable();
    (
        samples[samples.len() / 2].max(1),
        maximum,
        max_settle_frames,
    )
}
