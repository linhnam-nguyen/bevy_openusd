//! Local projection of authoritative viewport events for reference adapters.
//!
//! The Frost reference UI consumes this projection instead of reading Bevy
//! feature resources directly. Remote frontends apply the same public events
//! in their own event reducer.

use std::collections::{HashMap, HashSet};

use bevy::prelude::Resource;
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, EditorStateReadModel, PrimNodeReadModel, SceneAnchor,
    SceneChildrenPage, SceneSearchMatch, StageLoadState, ViewportEvent, ViewportEventEnvelope,
    ViewportReadModel,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScenePageKey {
    parent: Option<SceneAnchor>,
    page: u32,
    page_size: u32,
}

/// A paged scene request that a local reference adapter must send through the
/// same public command as the remote frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScenePageRequest {
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
}

impl ScenePageRequest {
    fn key(&self) -> ScenePageKey {
        ScenePageKey {
            parent: self.parent.clone(),
            page: self.page,
            page_size: self.page_size,
        }
    }
}

#[derive(Debug, Clone)]
struct SceneSearchState {
    request_id: String,
    query: String,
    matches: Vec<SceneSearchMatch>,
    total: u32,
    has_more: bool,
}

/// Latest authoritative viewport snapshot as reduced from emitted events.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ViewportReadModelState {
    snapshot: Option<ViewportReadModel>,
    scene_nodes: HashMap<SceneAnchor, PrimNodeReadModel>,
    loaded_scene_pages: HashSet<ScenePageKey>,
    requested_scene_pages: HashSet<ScenePageKey>,
    pending_scene_pages: Vec<ScenePageRequest>,
    search: Option<SceneSearchState>,
    editor: EditorStateReadModel,
}

impl ViewportReadModelState {
    /// Returns the latest snapshot after the render server has published one.
    pub(crate) fn snapshot(&self) -> Option<&ViewportReadModel> {
        self.snapshot.as_ref()
    }

    /// Returns the currently loaded portion of the logical scene tree in a
    /// deterministic order. Nodes are public identities, never ECS entities.
    pub(crate) fn scene_nodes(&self) -> Vec<PrimNodeReadModel> {
        let mut nodes: Vec<_> = self.scene_nodes.values().cloned().collect();
        nodes.sort_by(|left, right| {
            left.anchor
                .prim_path
                .cmp(&right.anchor.prim_path)
                .then_with(|| {
                    left.anchor
                        .instance_context
                        .cmp(&right.anchor.instance_context)
                })
        });
        nodes
    }

    /// Starts a server-authoritative scene search. Only the matching response
    /// is retained, so stale worker results cannot overwrite newer text.
    pub(crate) fn begin_search(&mut self, request_id: String, query: String) {
        self.search = Some(SceneSearchState {
            request_id,
            query,
            matches: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    /// Clears only the transient search projection, not the loaded tree.
    pub(crate) fn clear_search(&mut self) {
        self.search = None;
    }

    /// Returns the latest accepted server-side search matches.
    pub(crate) fn search_results(&self) -> &[SceneSearchMatch] {
        self.search
            .as_ref()
            .map(|search| search.matches.as_slice())
            .unwrap_or_default()
    }

    /// Returns the authoritative count and continuation flag for the active
    /// server-side search, if any.
    pub(crate) fn search_status(&self) -> Option<(u32, bool)> {
        self.search
            .as_ref()
            .map(|search| (search.total, search.has_more))
    }

    pub(crate) fn editor_state(&self) -> &EditorStateReadModel {
        &self.editor
    }

    /// Returns the next server-side search page without changing the active
    /// query. The caller sends it through `ViewportCommand`.
    pub(crate) fn next_search_page(&self) -> Option<(String, u32)> {
        self.search.as_ref().and_then(|search| {
            search
                .has_more
                .then(|| (search.query.clone(), search.matches.len() as u32))
        })
    }

    /// Marks a node as expanded. The initial child page is queued only once;
    /// subsequent pages are requested serially as each response arrives.
    pub(crate) fn request_scene_children(&mut self, parent: SceneAnchor) {
        self.queue_scene_page(Some(parent), 0, DEFAULT_SCENE_PAGE_SIZE);
    }

    /// Drains requests that must be sent through `ViewportCommand`.
    pub(crate) fn take_scene_page_requests(&mut self) -> Vec<ScenePageRequest> {
        std::mem::take(&mut self.pending_scene_pages)
    }

    /// Applies an event exactly as a frontend event reducer would.
    pub(crate) fn apply(&mut self, envelope: &ViewportEventEnvelope) {
        match &envelope.event {
            ViewportEvent::Snapshot { state } => self.apply_snapshot(state.as_ref().clone()),
            ViewportEvent::SceneChildren { page } => self.apply_scene_children(page),
            ViewportEvent::SearchResults {
                query,
                offset,
                total,
                matches,
                has_more,
            } => self.apply_search_results(
                envelope.request_id.as_deref(),
                query,
                *offset,
                *total,
                matches,
                *has_more,
            ),
            ViewportEvent::Ready { .. }
            | ViewportEvent::CameraTransitionStarted { .. }
            | ViewportEvent::CommandRejected { .. } => {}
            ViewportEvent::StageLoadStateChanged { state } => {
                if !matches!(state, StageLoadState::Ready) {
                    self.clear_scene();
                }
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.stage.loaded = matches!(state, StageLoadState::Ready);
                }
            }
            ViewportEvent::SelectionChanged { selection } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.selection = selection.clone();
                }
            }
            ViewportEvent::CameraSourceChanged { source } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.camera_source = source.clone();
                }
            }
            ViewportEvent::CameraOrientationChanged { orientation } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.camera_orientation = *orientation;
                }
            }
            ViewportEvent::CameraStandardViewStarted { .. } => {}
            ViewportEvent::TimelineChanged { timeline } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.timeline = timeline.clone();
                }
            }
            ViewportEvent::PresentationChanged { presentation } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.presentation = presentation.clone();
                }
            }
            ViewportEvent::ViewerSettingsChanged { settings } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.viewer_settings = settings.clone();
                }
            }
            ViewportEvent::PhysicsChanged { running } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.physics_running = *running;
                }
            }
            ViewportEvent::PrimVisibilityChanged { target, visible } => {
                self.apply_subtree_visibility(target, *visible);
                if let Some(snapshot) = &mut self.snapshot
                    && let Some(node) = snapshot
                        .scene
                        .prims
                        .iter_mut()
                        .find(|node| node.anchor == *target)
                {
                    node.visible = *visible;
                }
            }
            ViewportEvent::EditorCommandCompleted { state, .. }
            | ViewportEvent::RuntimeMutationBatchAccepted { state, .. } => {
                self.editor = state.clone();
            }
            ViewportEvent::EditorPrimState { .. }
            | ViewportEvent::EditorStageExportChunk { .. } => {}
        }
    }

    fn apply_snapshot(&mut self, snapshot: ViewportReadModel) {
        if !snapshot.stage.loaded {
            self.clear_scene();
            self.snapshot = Some(snapshot);
            return;
        }

        let roots: HashSet<_> = snapshot
            .scene
            .prims
            .iter()
            .filter(|node| node.parent.is_none())
            .map(|node| node.anchor.clone())
            .collect();
        // A snapshot carries only the first root page. Do not discard roots
        // fetched through later pages unless this snapshot is complete.
        if snapshot.scene.total_roots <= snapshot.scene.prims.len() as u32 {
            self.scene_nodes
                .retain(|anchor, node| node.parent.is_some() || roots.contains(anchor));
            self.remove_orphaned_scene_nodes();
        }
        for node in &snapshot.scene.prims {
            self.scene_nodes.insert(node.anchor.clone(), node.clone());
        }

        let root_page_size = snapshot.scene.root_page_size.max(1);
        self.loaded_scene_pages.insert(ScenePageKey {
            parent: None,
            page: 0,
            page_size: root_page_size,
        });
        if snapshot.scene.total_roots > snapshot.scene.prims.len() as u32 {
            self.queue_scene_page(None, 1, root_page_size);
        }
        self.snapshot = Some(snapshot);
    }

    fn apply_scene_children(&mut self, page: &SceneChildrenPage) {
        let key = ScenePageKey {
            parent: page.parent.clone(),
            page: page.page,
            page_size: page.page_size.max(1),
        };
        self.requested_scene_pages.remove(&key);
        self.loaded_scene_pages.insert(key);
        for node in &page.nodes {
            self.scene_nodes.insert(node.anchor.clone(), node.clone());
        }

        let next_page = page.page.saturating_add(1);
        if next_page.saturating_mul(page.page_size.max(1)) < page.total {
            self.queue_scene_page(page.parent.clone(), next_page, page.page_size.max(1));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_search_results(
        &mut self,
        request_id: Option<&str>,
        query: &str,
        offset: u32,
        total: u32,
        matches: &[SceneSearchMatch],
        has_more: bool,
    ) {
        let Some(search) = &mut self.search else {
            return;
        };
        if request_id != Some(search.request_id.as_str())
            || query != search.query
            || offset != search.matches.len() as u32
        {
            return;
        }
        search.matches.extend_from_slice(matches);
        search.total = total;
        search.has_more = has_more;
    }

    fn queue_scene_page(&mut self, parent: Option<SceneAnchor>, page: u32, page_size: u32) {
        let request = ScenePageRequest {
            parent,
            page,
            page_size: page_size.max(1),
        };
        let key = request.key();
        if self.loaded_scene_pages.contains(&key) || !self.requested_scene_pages.insert(key) {
            return;
        }
        self.pending_scene_pages.push(request);
    }

    fn apply_subtree_visibility(&mut self, target: &SceneAnchor, visible: bool) {
        let mut affected = HashSet::from([target.clone()]);
        let mut pending = vec![target.clone()];
        while let Some(parent) = pending.pop() {
            for node in self.scene_nodes.values() {
                if node.parent.as_ref() == Some(&parent) && affected.insert(node.anchor.clone()) {
                    pending.push(node.anchor.clone());
                }
            }
        }
        for node in self.scene_nodes.values_mut() {
            if affected.contains(&node.anchor) {
                node.visible = visible;
            }
        }
    }

    fn remove_orphaned_scene_nodes(&mut self) {
        loop {
            let orphaned: Vec<_> = self
                .scene_nodes
                .iter()
                .filter_map(|(anchor, node)| {
                    node.parent
                        .as_ref()
                        .filter(|parent| !self.scene_nodes.contains_key(*parent))
                        .map(|_| anchor.clone())
                })
                .collect();
            if orphaned.is_empty() {
                return;
            }
            for anchor in orphaned {
                self.scene_nodes.remove(&anchor);
            }
        }
    }

    fn clear_scene(&mut self) {
        self.scene_nodes.clear();
        self.loaded_scene_pages.clear();
        self.requested_scene_pages.clear();
        self.pending_scene_pages.clear();
        self.search = None;
    }
}

#[cfg(test)]
mod tests;
