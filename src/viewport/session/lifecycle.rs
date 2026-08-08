//! Native local-file lifecycle for the active USD session.
//!
//! The current viewer host resolves a CLI path and owns the Bevy app setup.
//! This module owns the corresponding stage asset's load, reload, and
//! materialization lifecycle. A future platform host can supply a cached
//! revision through the same session boundary.

use bevy::asset::{AssetEvent, AssetServer, Assets, Handle, LoadState};
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use bevy::scene::ScenePatchInstance;
use usd_bevy::{UsdAsset, UsdLoaderSettings};

use super::{
    LoadRequest, LoaderTuning, ReloadRequest, RequestedAsset, Spawned, StageHandle, StageInfo,
};

/// Starts loading the requested USD stage using the current loader tuning.
pub(crate) fn load_stage(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    requested: Res<RequestedAsset>,
    tuning: Res<LoaderTuning>,
) {
    // Pass the absolute asset-root directory as a search path so openusd
    // can chase sibling references like `@./greenhouse/front.usdc@`.
    let search = vec![requested.root.clone()];
    let kind_collapse = std::env::var("BEVY_OPENUSD_KIND_COLLAPSE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    let curve_radius = tuning.curves.default_radius;
    let curve_rings = tuning.curves.ring_segments;
    let point_scale = tuning.curves.point_scale;
    let variant_selections = tuning.to_variant_selections();
    let handle: Handle<UsdAsset> = asset_server
        .load_builder()
        .with_settings(move |s: &mut UsdLoaderSettings| {
            s.search_paths = search.clone();
            s.kind_collapse = kind_collapse;
            s.curve_default_radius = curve_radius;
            s.curve_ring_segments = curve_rings;
            s.point_scale = point_scale;
            s.variant_selections = variant_selections.clone();
        })
        .load(requested.name.clone());
    commands.insert_resource(StageHandle(handle));
    info!(
        "queued asset load: {} (search paths: {:?}, kind_collapse={}, curve_radius={}, curve_rings={}, point_scale={}, variants={})",
        requested.name,
        requested.root,
        kind_collapse,
        curve_radius,
        curve_rings,
        point_scale,
        tuning.variants.len()
    );
}

/// React to `AssetEvent::<UsdAsset>::Modified` (fired by Bevy's file
/// watcher when the source USD changes on disk). Despawn the existing
/// ScenePatchInstance(s) and rerun the spawn path — the new scene handle inside
/// UsdAsset will differ, so `Spawned` gets reset.
/// Reloads the active USD stage after a file event or explicit reload request.
pub(crate) fn handle_usd_hot_reload(
    mut events: MessageReader<AssetEvent<UsdAsset>>,
    mut commands: Commands,
    mut stage: Option<ResMut<StageHandle>>,
    scene_roots: Query<Entity, With<ScenePatchInstance>>,
    mut spawned: ResMut<Spawned>,
    mut reload: ResMut<ReloadRequest>,
    asset_server: Res<AssetServer>,
    requested: Res<RequestedAsset>,
    tuning: Res<LoaderTuning>,
    mut usd_assets: ResMut<bevy::asset::Assets<UsdAsset>>,
) {
    let Some(stage) = stage.as_deref_mut() else {
        return;
    };

    // Automatic path: fired by Bevy's file watcher (when re-enabled).
    for event in events.read() {
        if matches!(event, AssetEvent::Modified { id } if *id == stage.0.id()) {
            info!("hot-reload: UsdAsset modified, respawning scene");
            for entity in &scene_roots {
                commands.entity(entity).despawn();
            }
            spawned.0 = false;
        }
    }

    // Manual path: R keypress or UI button flipped ReloadRequest.
    if reload.requested {
        reload.requested = false;
        for entity in &scene_roots {
            commands.entity(entity).despawn();
        }
        // Drop the previously-loaded UsdAsset so its handles can be
        // freed when the new load replaces stage.0.
        let old_id = stage.0.id();
        usd_assets.remove(old_id);

        let search = vec![requested.root.clone()];
        let kind_collapse = std::env::var("BEVY_OPENUSD_KIND_COLLAPSE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
            .unwrap_or(false);
        let radius = tuning.curves.default_radius;
        let rings = tuning.curves.ring_segments;
        let point_scale = tuning.curves.point_scale;
        let variant_selections = tuning.to_variant_selections();

        // Bevy's AssetServer caches handles by asset path. Calling
        // `load_with_settings` a second time with the same path just
        // returns the prior handle without re-running the loader
        // closure — even when our closure captures different settings.
        // Route each reload through a variant-keyed copy sitting NEXT
        // TO the source (so the asset-root gate Bevy enforces still
        // passes, and openusd's sibling-reference resolution keeps
        // working). Per-selection hashing makes the asset path unique,
        // forcing a fresh loader run.
        let source_path = requested.root.join(&requested.name);
        let variant_basename = unique_variant_basename(&source_path, &variant_selections);
        let variant_fs_path = requested.root.join(&variant_basename);
        if let Err(err) = ensure_variant_copy(&source_path, &variant_fs_path) {
            error!(
                "hot-reload: failed to materialize variant-keyed copy {}: {err}",
                variant_fs_path.display()
            );
            return;
        }

        info!(
            "hot-reload: manual reload of {} via {} (curve_radius={radius:.4}, curve_rings={rings}, point_scale={point_scale:.2}, variants={})",
            requested.name,
            variant_basename,
            variant_selections.len()
        );
        let handle: Handle<UsdAsset> = asset_server
            .load_builder()
            .with_settings(move |s: &mut UsdLoaderSettings| {
                s.search_paths = search.clone();
                s.kind_collapse = kind_collapse;
                s.curve_default_radius = radius;
                s.curve_ring_segments = rings;
                s.point_scale = point_scale;
                s.variant_selections = variant_selections.clone();
            })
            // Relative asset name so Bevy's asset-root gate accepts it.
            .load(variant_basename.clone());
        stage.0 = handle;
        spawned.0 = false;
    }
}

/// Spawns the loaded USD scene and retires the fallback sun when appropriate.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<Assets<UsdAsset>>,
    stage: Res<StageHandle>,
    requested: Res<RequestedAsset>,
    mut spawned: ResMut<Spawned>,
    mut info: ResMut<StageInfo>,
) {
    if spawned.0 {
        return;
    }
    match asset_server.get_load_state(&stage.0) {
        Some(LoadState::Loaded) => {
            // During a variant reload we remove the previous UsdAsset
            // from `Assets<UsdAsset>` to force the loader to re-run.
            // The AssetServer's load-state tracker can still report
            // `Loaded` for a frame or two before the new load actually
            // populates storage — treat that gap as "still loading".
            let Some(asset) = assets.get(&stage.0) else {
                return;
            };
            info!(
                "loaded UsdAsset: default_prim={:?}, layer_count={}, variants={} prims",
                asset.default_prim,
                asset.layer_count,
                asset.variants.len()
            );
            info.default_prim = asset.default_prim.clone();
            info.layer_count = asset.layer_count;
            info.variant_count = asset.variants.values().map(|sets| sets.len()).sum();
            info.lights_directional = asset.light_tally.directional;
            info.lights_point = asset.light_tally.point;
            info.lights_spot = asset.light_tally.spot;
            info.lights_dome = asset.light_tally.dome;
            info.instance_prim_count = asset.instance_prim_count;
            info.instance_prototype_reuses = asset.instance_prototype_reuses;
            info.animated_prim_count = asset.animated_prims.len();
            info.skeleton_count = asset.skeletons.len();
            info.skel_root_count = asset.skel_roots.len();
            info.skel_binding_count = asset.skel_bindings.len();
            info.render_settings_count = asset.render_settings.len();
            info.render_product_count = asset.render_products.len();
            info.render_var_count = asset.render_vars.len();
            let primary = asset.render_settings.first();
            info.render_primary_resolution = primary.and_then(|s| s.resolution);
            info.render_primary_path = primary.map(|s| s.path.clone());
            info.rigid_body_count = asset.rigid_body_prims.len();
            info.physics_scene_count = asset.physics_scene_prims.len();
            info.joint_count = asset.joints.len();
            info.custom_attr_prim_count = asset.custom_attrs.len();
            info.custom_layer_data_entries = asset.custom_layer_data.len();
            info.subdivision_prim_count = asset.subdivision_prims.len();
            info.light_linked_count = asset.light_linking_prims.len();
            info.clip_prim_count = asset.clip_sets.len();
            commands.spawn(ScenePatchInstance(asset.scene.clone()));
            spawned.0 = true;
            sweep_variant_tempfiles_in_root(&requested.root);
        }
        Some(LoadState::Failed(err)) => {
            error!("UsdAsset load failed: {err}");
            spawned.0 = true;
        }
        _ => {}
    }
}

/// Handle the Browse-USD file picker result by re-exec'ing the viewer
/// binary with the picked path as `argv[1]`.
///
/// Bevy's `AssetServer` is configured at startup with one root directory
/// (`AssetPlugin.file_path`) and doesn't support changing that root
/// after the App is running — any subsequent `asset_server.load(abs_path)`
/// call resolves against the original root, silently mis-loading or
/// failing when the picked file lives elsewhere. Spawning a fresh
/// process bypasses that lock entirely: the new instance reads
/// `argv[1]` in `resolve_requested_asset`, sets the AssetPlugin's
/// `file_path` to the picked file's parent before the App is built,
/// and loads cleanly.
/// Applies a file-picker request by replacing the requested asset and restarting load.
pub(crate) fn apply_load_request(mut req: ResMut<LoadRequest>) {
    let Some(new_path) = req.path.take() else {
        return;
    };

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            error!("Browse: cannot resolve current_exe to re-launch: {e}");
            return;
        }
    };

    info!(
        "Browse: re-launching {} with {}",
        exe.display(),
        new_path.display()
    );

    match std::process::Command::new(&exe).arg(&new_path).spawn() {
        Ok(_) => {
            // New viewer is up; exit cleanly so we don't sit alongside it.
            std::process::exit(0);
        }
        Err(e) => {
            error!("Browse: failed to spawn new viewer process: {e}");
        }
    }
}

/// Wipe stale `.bevy_openusd_variant_<hash>.usda` copies left in the
/// asset root by prior viewer runs. Fires once at startup before
/// `load_stage` queues the initial load, so the subsequent fresh
/// copies are the only ones on disk.
/// Removes stale temporary variant copies before stage loading begins.
pub(crate) fn sweep_variant_tempfiles(requested: Res<RequestedAsset>) {
    sweep_variant_tempfiles_in_root(&requested.root);
}

/// Deletes this viewer's prefixed temporary USD variant files in one directory.
fn sweep_variant_tempfiles_in_root(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            continue;
        };
        if name.starts_with(".bevy_openusd_variant_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Build a per-variant-selection asset basename. Bevy's AssetServer
/// caches by asset path alone, so each distinct selection set needs a
/// distinct path to force a fresh loader run. The basename sits
/// alongside the real asset file so Bevy's asset-root gate accepts it.
/// Produces a collision-resistant basename for a temporary variant copy.
fn unique_variant_basename(
    source: &std::path::Path,
    selections: &[usd_bevy::VariantSelection],
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    for sel in selections {
        sel.prim_path.hash(&mut h);
        sel.set_name.hash(&mut h);
        sel.option.hash(&mut h);
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("usda");
    format!(".bevy_openusd_variant_{:016x}.{ext}", h.finish())
}

/// Ensure `dest` exists and mirrors `source`'s bytes. We re-copy only
/// when the destination is missing or stale compared to the source's
/// modification time.
/// Creates or refreshes a temporary copy only when its source has changed.
fn ensure_variant_copy(source: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    let needs_copy = match (source.metadata(), dest.metadata()) {
        (Ok(s), Ok(d)) => match (s.modified().ok(), d.modified().ok()) {
            (Some(s_mt), Some(d_mt)) => s_mt > d_mt,
            _ => true,
        },
        _ => true,
    };
    if needs_copy {
        std::fs::copy(source, dest)?;
    }
    Ok(())
}
