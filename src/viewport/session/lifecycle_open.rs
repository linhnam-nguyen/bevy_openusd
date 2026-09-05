use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::LiveStage;

use super::{Spawned, StageHandle, StagePresentationContext, lifecycle_invalidation};

pub(super) fn open_stage(world: &mut World, path: std::path::PathBuf) {
    let presentation = world
        .get_resource::<StagePresentationContext>()
        .cloned()
        .unwrap_or_default();
    world.remove_non_send::<LiveStage>();
    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
    {
        cache.clear_active_archives();
    }
    lifecycle_invalidation::reset_derived_state(world, 0);
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    world.resource_mut::<Spawned>().0 = false;

    info!("opening USD stage: {}", path.display());
    let path_string = path.to_string_lossy().into_owned();
    match Stage::open(&path_string) {
        Ok(stage) => {
            let archive_paths = match usd_bevy::route::material::archive_paths_for_stage(
                &stage, &path,
            ) {
                Ok(paths) => paths,
                Err(error) => {
                    warn!(
                        "could not derive active USDZ packages for {}; embedded textures will use source fallback: {error:#}",
                        path.display()
                    );
                    Vec::new()
                }
            };
            if let Some(mut cache) =
                world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
            {
                cache.replace_active_archives(archive_paths);
            }
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
