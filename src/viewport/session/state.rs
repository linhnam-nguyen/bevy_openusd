//! Session-owned state for the currently opened live USD stage.

use bevy::prelude::Resource;
use std::collections::HashMap;
use std::path::PathBuf;

/// Marker and error state for the active stage request.
///
/// The stage itself is held by `usd_bevy::LiveStage` as a non-send resource;
/// this resource only keeps the requested path and an optional open error so
/// the protocol can distinguish loading from failure.
#[derive(Resource, Debug, Clone)]
pub(crate) struct StageHandle {
    pub(crate) path: PathBuf,
    pub(crate) error: Option<String>,
}

/// Whether the live stage has projected at least one real prim into Bevy.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub(crate) struct Spawned(pub(crate) bool);

/// Local filesystem source requested for the current viewport session.
#[derive(Resource, Debug, Clone)]
pub(crate) struct RequestedAsset {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
}

#[derive(Resource, Default, Debug, Clone)]
pub struct StageInfo {
    /// Project activation generation that owns the current Stage snapshot.
    /// Zero denotes a stage opened outside the Project activation protocol.
    pub activation_generation: u64,
    pub path: String,
    pub default_prim: Option<String>,
    pub layer_count: usize,
    pub variant_count: usize,
    pub lights_directional: usize,
    pub lights_point: usize,
    pub lights_spot: usize,
    pub lights_dome: usize,
    pub instance_prim_count: usize,
    pub instance_prototype_reuses: usize,
    pub animated_prim_count: usize,
    pub skeleton_count: usize,
    pub skel_root_count: usize,
    pub skel_binding_count: usize,
    pub render_settings_count: usize,
    pub render_product_count: usize,
    pub render_var_count: usize,
    pub render_primary_resolution: Option<[i32; 2]>,
    pub render_primary_path: Option<String>,
    pub rigid_body_count: usize,
    pub physics_scene_count: usize,
    pub joint_count: usize,
    pub custom_attr_prim_count: usize,
    pub custom_layer_data_entries: usize,
    pub subdivision_prim_count: usize,
    pub light_linked_count: usize,
    pub clip_prim_count: usize,
    pub variants: HashMap<String, Vec<VariantSetInfo>>,
    pub cameras: Vec<StageCameraInfo>,
    pub skel_animation_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct VariantSetInfo {
    pub name: String,
    pub selection: Option<String>,
    /// The current OpenUSD binding exposes effective selections. Options are
    /// left empty until variant-child enumeration is promoted to its public
    /// API; authoring still goes through `usd_bevy::authoring::set_variant`.
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StageCameraInfo {
    pub path: String,
    pub data: StageCameraData,
}

#[derive(Debug, Clone)]
pub struct StageCameraData {
    pub focal_length_mm: Option<f32>,
    pub projection: Option<StageCameraProjection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageCameraProjection {
    Perspective,
    Orthographic,
}

/// Live editor controls retained by the viewport protocol. Curve values are
/// kept as presentation state; the current route implementation owns mesh
/// construction and does not require a loader rebuild.
#[derive(Resource, Debug, Clone, Default)]
pub struct LoaderTuning {
    pub curves: CurveTuning,
    pub variants: HashMap<(String, String), String>,
}

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

/// Flipped by the reload command or the native `R` shortcut.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ReloadRequest {
    pub requested: bool,
}

/// Re-launch request from the native file picker.
#[derive(Resource, Default, Debug, Clone)]
pub struct LoadRequest {
    pub path: Option<PathBuf>,
}
