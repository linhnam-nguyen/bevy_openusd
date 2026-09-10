//! Session-owned state for the currently opened live USD stage.

use bevy::prelude::Resource;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache_contract::{SceneCacheActivation, SceneCacheEntry, SceneCacheState};

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

/// Manifest-backed presentation context for the currently activated stage.
/// The stage path remains diagnostic state; this resource supplies the typed
/// Project identity needed to label the semantic hierarchy.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StagePresentationContext {
    pub(crate) root_path: Option<String>,
    pub(crate) root_name: Option<String>,
    pub(crate) target_names: HashMap<(String, String), String>,
}

impl StagePresentationContext {
    pub(crate) fn from_project(
        context: crate::project::service::ProjectStagePresentationContext,
    ) -> Self {
        Self {
            root_path: context.root_path,
            root_name: context.root_name,
            target_names: context.target_names,
        }
    }

    pub(crate) fn target_name(&self, kind: &str, id: &str) -> Option<&str> {
        self.target_names
            .get(&(kind.to_owned(), id.to_owned()))
            .map(String::as_str)
    }
}

/// Metadata-first Scene activation published before the canonical Stage has
/// produced a Bevy projection. The entries retain cache-owned identity and
/// bounds while OpenUSD remains the authority for later source projection.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct SceneCachePresentation {
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) state: SceneCacheState,
    pub(crate) entries: Vec<SceneCacheEntry>,
}

impl SceneCachePresentation {
    pub(crate) fn from_activation(activation: &SceneCacheActivation) -> Self {
        Self {
            scene_id: activation.descriptor.scene_id,
            generation: activation.descriptor.generation,
            state: activation.descriptor.state,
            entries: activation.index.entries.clone(),
        }
    }
}

/// Background strong source check for a cache-first Scene activation. The
/// result is consumed on the Bevy main world so stale cache state can never
/// replace canonical Stage/LiveStage authority.
#[derive(Resource)]
pub(crate) struct PendingSceneCacheRevalidation {
    pub(crate) project_root: PathBuf,
    pub(crate) scene_id: SceneId,
    pub(crate) activation_generation: u64,
    pub(crate) scene_generation: u64,
    pub(crate) expected_hash: Option<HashDigest>,
    pub(crate) config_hash: HashDigest,
    pub(crate) result: Arc<Mutex<Option<Result<HashDigest, String>>>>,
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
