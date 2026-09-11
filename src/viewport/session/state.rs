//! Session-owned state for the currently opened live USD stage.

use bevy::prelude::Resource;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usd_model::HashDigest;
use usd_project::SceneId;

use crate::project::cache_contract::{
    SceneCacheActivation, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheState,
};

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

/// Generation-owned derived metadata for the active Scene snapshot.
///
/// `StageInfo` mirrors this state for the existing viewport read model, but it
/// is not the authority for asynchronous prim-count results. Every inspection
/// result must match this identity before it can update the Scene-owned value.
#[derive(Resource, Clone, Debug, Eq, PartialEq)]
pub(crate) struct SceneDerivedMetadata {
    pub(crate) scene_id: Option<SceneId>,
    pub(crate) cache_project_root: Option<PathBuf>,
    pub(crate) cache_descriptor: Option<SceneCacheDescriptorV3>,
    pub(crate) cache_generation: Option<u64>,
    pub(crate) cache_state: Option<SceneCacheState>,
    pub(crate) path: PathBuf,
    pub(crate) activation_generation: u64,
    pub(crate) session_id: Option<u64>,
    pub(crate) prim_count: usize,
    pub(crate) prim_count_ready: bool,
    pub(crate) prim_count_pending: bool,
    pub(crate) prim_count_attempts: u8,
    pub(crate) prim_count_terminal: bool,
    pub(crate) prim_count_error: Option<String>,
}

impl SceneDerivedMetadata {
    pub(crate) fn uncached(path: PathBuf) -> Self {
        Self {
            scene_id: None,
            cache_project_root: None,
            cache_descriptor: None,
            cache_generation: None,
            cache_state: None,
            path,
            activation_generation: 0,
            session_id: None,
            prim_count: 0,
            prim_count_ready: false,
            prim_count_pending: false,
            prim_count_attempts: 0,
            prim_count_terminal: false,
            prim_count_error: None,
        }
    }

    pub(crate) fn from_activation(
        path: PathBuf,
        activation_generation: u64,
        owner: Option<(PathBuf, SceneId)>,
        cache: Option<&SceneCacheActivation>,
        descriptor: Option<SceneCacheDescriptorV3>,
    ) -> Self {
        let descriptor = descriptor.or_else(|| cache.map(|activation| activation.descriptor.clone()));
        let scene_id = owner
            .as_ref()
            .map(|(_, scene_id)| scene_id.clone())
            .or_else(|| descriptor.as_ref().map(|descriptor| descriptor.scene_id.clone()));
        let cache_generation = descriptor.as_ref().map(|descriptor| descriptor.generation);
        let cache_state = descriptor.as_ref().map(|descriptor| descriptor.state);
        let prim_count = descriptor
            .as_ref()
            .map_or(0, |descriptor| descriptor.prim_count as usize);
        let ready = descriptor.as_ref().is_some_and(|descriptor| {
            descriptor.prim_count_ready || descriptor.state == SceneCacheState::Ready
        });
        Self {
            scene_id,
            cache_project_root: owner.as_ref().map(|(root, _)| root.clone()),
            cache_descriptor: descriptor,
            cache_generation,
            cache_state,
            path,
            activation_generation,
            session_id: None,
            prim_count,
            prim_count_ready: ready,
            prim_count_pending: false,
            prim_count_attempts: 0,
            prim_count_terminal: false,
            prim_count_error: None,
        }
    }

    pub(crate) fn bind_session(&mut self, session_id: u64) {
        if self.session_id == Some(session_id) {
            return;
        }
        self.session_id = Some(session_id);
        self.prim_count_pending = false;
        self.prim_count_attempts = 0;
        self.prim_count_terminal = false;
        self.prim_count_error = None;
        if !self.cache_descriptor.as_ref().is_some_and(|descriptor| {
            descriptor.prim_count_ready || descriptor.state == SceneCacheState::Ready
        }) {
            self.prim_count_ready = false;
        }
    }
}

#[derive(Resource, Default, Debug, Clone)]
pub struct StageInfo {
    /// Project activation generation that owns the current Stage snapshot.
    /// Zero denotes a stage opened outside the Project activation protocol.
    pub activation_generation: u64,
    pub path: String,
    /// Metadata-only prim count. Cache-first activation supplies this from the
    /// Scene descriptor; uncached activation fills it asynchronously.
    pub prim_count: usize,
    pub prim_count_ready: bool,
    pub prim_count_pending: bool,
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
