//! Native viewer projection scheduling defaults.

use std::time::Duration;

use bevy::prelude::App;
use usd_bevy::ProjectionBudget;

pub(super) fn configure(app: &mut App) {
    app.insert_resource(ProjectionBudget::bounded(32, Duration::from_millis(8)));
}
