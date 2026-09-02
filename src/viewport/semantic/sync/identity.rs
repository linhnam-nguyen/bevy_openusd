use bevy::prelude::World;
use usd_bevy::StageChangeBatch;

use crate::viewport::session::StageInfo;

pub(super) fn activation_generation(world: &World) -> u64 {
    world
        .get_resource::<StageInfo>()
        .map_or(0, |info| info.activation_generation)
}

pub(super) fn resync_root_count(batch: Option<&StageChangeBatch>) -> usize {
    batch
        .filter(|batch| batch.has_resync())
        .map_or(0, |batch| batch.resync_roots().len())
}
