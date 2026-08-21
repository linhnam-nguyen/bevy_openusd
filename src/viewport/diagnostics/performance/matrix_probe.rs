use bevy::prelude::*;
use bevy_glacial::prelude::GroundGrid;

use super::runner::BenchmarkLaunchConfig;
use super::sample::{BenchmarkIdentity, RenderConfiguration, RenderMode};
use crate::viewport::scene::visualization::{EdgeOverlay, OriginalShadowEnabled};

pub(super) fn effective_renderer_configuration(world: &mut World) -> RenderConfiguration {
    let grid = world
        .get_resource::<GroundGrid>()
        .is_some_and(|ground_grid| ground_grid.visible);
    let render_mode = world
        .get_resource::<bevy::pbr::wireframe::WireframeConfig>()
        .map_or(RenderMode::Shaded, |config| {
            if config.global {
                RenderMode::Wireframe
            } else {
                RenderMode::Shaded
            }
        });
    let edges = effective_edge_configuration(world);
    let shadows = effective_shadow_configuration(world).unwrap_or(false);
    RenderConfiguration {
        grid,
        shadows,
        edges,
        render_mode,
        material_overrides: true,
    }
}

fn effective_edge_configuration(world: &mut World) -> bool {
    let mut overlays =
        world.query_filtered::<(&Visibility, &InheritedVisibility), With<EdgeOverlay>>();
    overlays
        .iter(world)
        .any(|(visibility, inherited)| !matches!(visibility, Visibility::Hidden) && inherited.get())
}

fn effective_shadow_configuration(world: &mut World) -> Option<bool> {
    let directional = {
        let mut lights = world.query::<(&DirectionalLight, &OriginalShadowEnabled)>();
        lights
            .iter(world)
            .find_map(|(light, authored)| authored.0.then_some(light.shadow_maps_enabled))
    };
    if directional.is_some() {
        return directional;
    }

    let point = {
        let mut lights = world.query::<(&PointLight, &OriginalShadowEnabled)>();
        lights
            .iter(world)
            .find_map(|(light, authored)| authored.0.then_some(light.shadow_maps_enabled))
    };
    if point.is_some() {
        return point;
    }

    let spot = {
        let mut lights = world.query::<(&SpotLight, &OriginalShadowEnabled)>();
        lights
            .iter(world)
            .find_map(|(light, authored)| authored.0.then_some(light.shadow_maps_enabled))
    };
    spot
}

pub(super) fn matrix_identity(world: &World, config: &BenchmarkLaunchConfig) -> BenchmarkIdentity {
    let (gpu_adapter, backend) = if let Some(adapter_info) =
        world.get_resource::<bevy::render::renderer::RenderAdapterInfo>()
    {
        (
            adapter_info.name.clone(),
            format!("{:?}", adapter_info.backend).to_lowercase(),
        )
    } else {
        ("unknown".to_owned(), "unknown".to_owned())
    };
    let scene_path = config
        .asset_path
        .clone()
        .unwrap_or_else(|| "no_stage".to_owned());
    let scene_label = std::path::Path::new(&scene_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&scene_path)
        .to_owned();
    let mut identity = BenchmarkIdentity::new(
        "M3-C6++",
        &scene_label,
        None,
        gpu_adapter,
        config.width,
        config.height,
        config.requested_fps,
    );
    identity.scene_path = scene_path;
    identity.backend = backend;
    identity.git_sha = benchmark_revision("USDHUB_GIT_SHA", option_env!("USDHUB_GIT_SHA"));
    identity.glacial_sha =
        benchmark_revision("USDHUB_GLACIAL_SHA", option_env!("USDHUB_GLACIAL_SHA"));
    identity
}

fn benchmark_revision(name: &str, compile_time: Option<&str>) -> String {
    std::env::var(name)
        .ok()
        .or_else(|| compile_time.map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}
