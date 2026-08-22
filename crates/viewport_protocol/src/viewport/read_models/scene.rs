use serde::{Deserialize, Serialize};

use super::identity::SceneAnchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    FrameTarget,
    FlyToTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimNodeReadModel {
    pub anchor: SceneAnchor,
    pub parent: Option<SceneAnchor>,
    pub label: String,
    pub visible: bool,
    pub has_children: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneReadModel {
    pub prims: Vec<PrimNodeReadModel>,
    #[serde(default)]
    pub total_prims: u32,
    #[serde(default)]
    pub total_roots: u32,
    #[serde(default)]
    pub root_page_size: u32,
}

/// A bounded page of direct children for one scene-tree parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneChildrenPage {
    pub parent: Option<SceneAnchor>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
    pub nodes: Vec<PrimNodeReadModel>,
}

/// One page that the frontend must load to reveal a search match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePageReference {
    pub parent: Option<SceneAnchor>,
    pub page: u32,
}

/// A compact server-side search match with enough information to reveal it in
/// a partially-loaded tree. The stable anchor is never reconstructed from the
/// display label, which may be truncated by the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneSearchMatch {
    pub anchor: SceneAnchor,
    pub parent: Option<SceneAnchor>,
    pub label: String,
    pub visible: bool,
    pub has_children: bool,
    pub reveal_pages: Vec<ScenePageReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurveTuning {
    pub default_radius: f32,
    pub ring_segments: u32,
    pub point_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReadModel {
    pub display_name: String,
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageLoadState {
    Idle,
    Loading,
    Ready,
    Failed { message: String },
}
