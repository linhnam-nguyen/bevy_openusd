//! Session-owned resources for the currently opened USD source.

use bevy::asset::Handle;
use bevy::prelude::Resource;
use std::path::PathBuf;
use usd_bevy::UsdAsset;

/// Handle of the Bevy asset backing the active USD session.
#[derive(Resource)]
pub(crate) struct StageHandle(pub(crate) Handle<UsdAsset>);

/// Whether the current stage asset has been projected into the Bevy world.
#[derive(Resource, Default)]
pub(crate) struct Spawned(pub(crate) bool);

/// Local filesystem source requested for the current viewport session.
///
/// This remains viewport-local: a future host will resolve a revision or
/// cache reference into the same source abstraction.
#[derive(Resource)]
pub(crate) struct RequestedAsset {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
}

#[derive(Resource, Default, Debug, Clone)]
pub struct StageInfo {
    pub path: String,
    pub default_prim: Option<String>,
    pub layer_count: usize,
    pub variant_count: usize,
    /// UsdLux totals captured at load time (M9).
    pub lights_directional: usize,
    pub lights_point: usize,
    pub lights_spot: usize,
    pub lights_dome: usize,
    /// Prims marked `instanceable = true` and, of those, how many were
    /// recognised as reuses of a prototype we'd already built (M14).
    pub instance_prim_count: usize,
    pub instance_prototype_reuses: usize,
    /// Prims whose xformOps carry timeSamples (M15).
    pub animated_prim_count: usize,
    /// UsdSkel totals (M16 read side).
    pub skeleton_count: usize,
    pub skel_root_count: usize,
    pub skel_binding_count: usize,
    /// UsdRender totals + primary resolution (M19 read side).
    pub render_settings_count: usize,
    pub render_product_count: usize,
    pub render_var_count: usize,
    pub render_primary_resolution: Option<[i32; 2]>,
    pub render_primary_path: Option<String>,
    /// UsdPhysics totals (M_LAST read side).
    pub rigid_body_count: usize,
    pub physics_scene_count: usize,
    pub joint_count: usize,
    /// Custom-attribute / customData / assetInfo coverage (M24).
    pub custom_attr_prim_count: usize,
    pub custom_layer_data_entries: usize,
    /// Subdivision-surface meshes (M25).
    pub subdivision_prim_count: usize,
    /// Lights with authored `light:link` rels (M26).
    pub light_linked_count: usize,
    /// Prims carrying `UsdClipsAPI` metadata (M27).
    pub clip_prim_count: usize,
}

/// Flipped to `true` by the keyboard handler (R key) or the UI panel to
/// request a force-reload of the current USD asset. The main-loop's
/// `handle_usd_hot_reload` system reacts on the next frame.
#[derive(Resource, Default, Debug, Clone)]
pub struct ReloadRequest {
    pub requested: bool,
}

/// Swap the loaded asset at runtime. Set by the Browse-USD file picker.
/// On the next frame, the viewer despawns current ScenePatchInstances + updates
/// the RequestedAsset + re-registers the AssetPlugin search roots.
#[derive(Resource, Default, Debug, Clone)]
pub struct LoadRequest {
    pub path: Option<PathBuf>,
}

/// Live knobs the viewer passes to `UsdLoaderSettings` on every load
/// / reload: curve + point rendering defaults plus any variant-selection
/// overrides authored in the Variants panel. Bundled into one Resource
/// so systems that need both (load_stage, handle_usd_hot_reload,
/// draw_panel) stay under Bevy's 16-param limit.
#[derive(Resource, Debug, Clone, Default)]
pub struct LoaderTuning {
    pub curves: CurveTuning,
    /// `(prim_path, set_name) → selected option`. Empty = honour the
    /// stage's authored selections.
    pub variants: std::collections::HashMap<(String, String), String>,
}

impl LoaderTuning {
    /// Converts the UI's map-based overrides into loader-facing selections.
    pub fn to_variant_selections(&self) -> Vec<usd_bevy::VariantSelection> {
        self.variants
            .iter()
            .map(
                |((prim_path, set_name), option)| usd_bevy::VariantSelection {
                    prim_path: prim_path.clone(),
                    set_name: set_name.clone(),
                    option: option.clone(),
                },
            )
            .collect()
    }
}

/// Curve / point rendering defaults. Not a Resource on its own — lives
/// inside [`LoaderTuning`]. Split so the rebuild-tuned-meshes system
/// can diff a lightweight `Copy` key without cloning the variant map.
#[derive(Debug, Clone, Copy)]
pub struct CurveTuning {
    pub default_radius: f32,
    pub ring_segments: u32,
    pub point_scale: f32,
}

impl Default for CurveTuning {
    fn default() -> Self {
        Self {
            default_radius: 0.02,
            ring_segments: 6,
            point_scale: 1.0,
        }
    }
}
