//! Renderer-side mesh preparation and buffer upload timing for mesh benchmarks.

use bevy::prelude::*;
use bevy::render::mesh::{RenderMesh, allocator::allocate_and_free_meshes};
use bevy::render::render_asset::{ExtractedAssets, RenderAsset, prepare_assets};
use bevy::render::{ExtractSchedule, MainWorld, Render, RenderApp};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Main-world aggregate for the renderer-side mesh preparation window.
///
/// The elapsed time covers Bevy's mesh allocator/upload system and the
/// `RenderMesh` preparation system. It is a CPU-side renderer measurement of
/// the work that queues GPU buffer writes; it does not claim GPU completion.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GeometryRenderPreparation {
    pub render_mesh_prepare_upload_ms: f64,
    pub render_prepared_meshes: u64,
    pub render_prepared_bytes: u64,
    pub render_prepare_windows: u64,
}

#[derive(Resource, Default)]
struct GeometryRenderPreparationWindow {
    started_at: Option<Instant>,
    prepared_meshes: u64,
    prepared_bytes: u64,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<GeometryRenderPreparation>();
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<GeometryRenderPreparation>()
        .init_resource::<GeometryRenderPreparationWindow>()
        .add_systems(ExtractSchedule, extract_geometry_render_preparation)
        .add_systems(
            Render,
            begin_geometry_render_preparation.before(allocate_and_free_meshes),
        )
        .add_systems(
            Render,
            finish_geometry_render_preparation.after(prepare_assets::<RenderMesh>),
        );
}

pub(super) fn snapshot(world: &World, enabled: bool) -> Option<GeometryRenderPreparation> {
    enabled
        .then(|| world.get_resource::<GeometryRenderPreparation>().copied())
        .flatten()
}

fn begin_geometry_render_preparation(
    mut window: ResMut<GeometryRenderPreparationWindow>,
    extracted: Res<ExtractedAssets<RenderMesh>>,
) {
    window.started_at = Some(Instant::now());
    window.prepared_meshes = extracted.extracted.len() as u64;
    window.prepared_bytes = extracted
        .extracted
        .iter()
        .filter_map(|(_, mesh)| <RenderMesh as RenderAsset>::byte_len(mesh))
        .map(|bytes| bytes as u64)
        .sum();
}

fn finish_geometry_render_preparation(
    mut window: ResMut<GeometryRenderPreparationWindow>,
    mut report: ResMut<GeometryRenderPreparation>,
) {
    let Some(started_at) = window.started_at.take() else {
        return;
    };
    let elapsed_ms = started_at.elapsed().as_secs_f64() * 1000.0;
    let prepared_meshes = window.prepared_meshes;
    let prepared_bytes = window.prepared_bytes;
    window.prepared_meshes = 0;
    window.prepared_bytes = 0;

    report.render_mesh_prepare_upload_ms += elapsed_ms;
    report.render_prepared_meshes += prepared_meshes;
    report.render_prepared_bytes += prepared_bytes;
    report.render_prepare_windows += 1;
}

fn extract_geometry_render_preparation(
    render_report: Res<GeometryRenderPreparation>,
    mut main_world: ResMut<MainWorld>,
) {
    let Some(mut main_report) = main_world.get_resource_mut::<GeometryRenderPreparation>() else {
        return;
    };
    *main_report = *render_report;
}
