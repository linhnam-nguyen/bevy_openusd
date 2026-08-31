use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::LiveStage;

use super::{Spawned, StageHandle, StagePresentationContext};

pub(super) fn open_stage(world: &mut World, path: std::path::PathBuf) {
    let presentation = world
        .get_resource::<StagePresentationContext>()
        .cloned()
        .unwrap_or_default();
    world.remove_non_send::<LiveStage>();
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    world.resource_mut::<Spawned>().0 = false;

    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        && !cache.archive_paths.contains(&path)
    {
        cache.archive_paths.push(path.clone());
    }

    info!("opening USD stage: {}", path.display());
    let path_string = path.to_string_lossy().into_owned();
    match Stage::open(&path_string) {
        Ok(stage) => {
            world.insert_resource(presentation);
            world.insert_non_send(LiveStage::new(stage));
        }
        Err(error) => {
            let message = format!("failed to open {}: {error:#}", path.display());
            error!("{message}");
            world.resource_mut::<StageHandle>().error = Some(message);
        }
    }
}
