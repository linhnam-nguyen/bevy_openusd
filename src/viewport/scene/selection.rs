//! Runtime selection state for the current Bevy projection.

use bevy::prelude::{Entity, Resource};

/// Selected Bevy entity. This remains an internal runtime detail; the future
/// platform boundary will translate it to a stable USD scene anchor.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectedPrim(pub Option<Entity>);
