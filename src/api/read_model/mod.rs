//! Local projection of authoritative viewport events for reference adapters.
//!
//! The Frost reference UI consumes this projection instead of reading Bevy
//! feature resources directly. Remote frontends apply the same public events
//! in their own event reducer.

use std::collections::{HashMap, HashSet};

use bevy::prelude::Resource;
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimClassificationFieldCataloguePage, BimPropertiesPage,
    BimPropertiesReadModel, BimPropertyGroupId, BimPropertyGroupReadModel, DEFAULT_SCENE_PAGE_SIZE,
    EditorStateReadModel, MAX_BIM_CLASSIFICATION_FIELDS, MAX_BIM_CLASSIFICATION_PAGES,
    MAX_BIM_PROPERTY_COUNT, MAX_BIM_PROPERTY_PAGES, PrimNodeReadModel, RequestId, SceneAnchor,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingBimRequest {
    request_id: RequestId,
    stage_epoch: u64,
    activation_generation: u64,
    selection_revision: u64,
    targets: Vec<SceneAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingCatalogueRequest {
    request_id: RequestId,
    stage_epoch: u64,
    activation_generation: u64,
}

#[derive(Debug, Clone)]
struct BimPropertiesPageAssembly {
    request_id: RequestId,
    stage_epoch: u64,
    activation_generation: u64,
    selection_revision: u64,
    targets: Vec<SceneAnchor>,
    page_count: u32,
    total_properties: u32,
    pages: HashMap<u32, BimPropertiesPage>,
}

#[derive(Debug, Clone)]
struct BimCataloguePageAssembly {
    request_id: RequestId,
    stage_epoch: u64,
    activation_generation: u64,
    semantic_revision: u64,
    page_count: u32,
    total_fields: u32,
    pages: HashMap<u32, BimClassificationFieldCataloguePage>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BimPropertiesLoadState<'a> {
    StageNotLoaded,
    NoSelection,
    NotRequested,
    Loading {
        received_pages: u32,
        total_pages: u32,
    },
    Error(&'a str),
    Ready(&'a BimPropertiesReadModel),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BimCatalogueLoadState<'a> {
    StageNotLoaded,
    NotRequested,
    Loading {
        received_pages: u32,
        total_pages: u32,
    },
    Error(&'a str),
    Ready(&'a BimClassificationFieldCatalogue),
}

/// Latest authoritative viewport snapshot as reduced from emitted events.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ViewportReadModelState {
    snapshot: Option<ViewportReadModel>,
    stage_epoch: u64,
    scene_nodes: HashMap<SceneAnchor, PrimNodeReadModel>,
    loaded_scene_pages: HashSet<ScenePageKey>,
    requested_scene_pages: HashSet<ScenePageKey>,
    pending_scene_pages: Vec<ScenePageRequest>,
    search: Option<SceneSearchState>,
    editor: EditorStateReadModel,
    bim_properties: Option<BimPropertiesReadModel>,
    pending_bim: Option<PendingBimRequest>,
    bim_properties_assembly: Option<BimPropertiesPageAssembly>,
    bim_properties_error: Option<String>,
    bim_classification_catalogue: Option<BimClassificationFieldCatalogue>,
    pending_catalogue: Option<PendingCatalogueRequest>,
    bim_catalogue_assembly: Option<BimCataloguePageAssembly>,
    bim_catalogue_error: Option<String>,
    catalogue_refresh_needed: Option<(u64, u64)>,
    last_error: Option<String>,
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

    pub(crate) fn stage_epoch(&self) -> u64 {
        self.stage_epoch
    }

    pub(crate) fn bim_properties(&self) -> Option<&BimPropertiesReadModel> {
        let props = self.bim_properties.as_ref()?;
        let snapshot = self.snapshot.as_ref()?;
        (snapshot.stage.loaded
            && !snapshot.selection.targets.is_empty()
            && props.targets == snapshot.selection.targets
            && props.selection_revision == snapshot.selection_revision)
            .then_some(props)
    }

    pub(crate) fn bim_properties_state(&self) -> BimPropertiesLoadState<'_> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return BimPropertiesLoadState::StageNotLoaded;
        };
        if !snapshot.stage.loaded {
            return BimPropertiesLoadState::StageNotLoaded;
        }
        if snapshot.selection.targets.is_empty() {
            return BimPropertiesLoadState::NoSelection;
        }
        if let Some(error) = self.bim_properties_error.as_deref() {
            return BimPropertiesLoadState::Error(error);
        }
        if self.pending_bim.is_some() {
            if let Some(assembly) = &self.bim_properties_assembly {
                return BimPropertiesLoadState::Loading {
                    received_pages: assembly.pages.len() as u32,
                    total_pages: assembly.page_count,
                };
            }
            return BimPropertiesLoadState::Loading {
                received_pages: 0,
                total_pages: 0,
            };
        }
        if let Some(props) = self.bim_properties() {
            return BimPropertiesLoadState::Ready(props);
        }
        BimPropertiesLoadState::NotRequested
    }

    pub(crate) fn bim_classification_catalogue(
        &self,
    ) -> Option<&BimClassificationFieldCatalogue> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .stage
            .loaded
            .then_some(self.bim_classification_catalogue.as_ref())
            .flatten()
    }

    pub(crate) fn bim_catalogue_state(&self) -> BimCatalogueLoadState<'_> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return BimCatalogueLoadState::StageNotLoaded;
        };
        if !snapshot.stage.loaded {
            return BimCatalogueLoadState::StageNotLoaded;
        }
        if let Some(error) = self.bim_catalogue_error.as_deref() {
            return BimCatalogueLoadState::Error(error);
        }
        if self.pending_catalogue.is_some() {
            if let Some(assembly) = &self.bim_catalogue_assembly {
                return BimCatalogueLoadState::Loading {
                    received_pages: assembly.pages.len() as u32,
                    total_pages: assembly.page_count,
                };
            }
            return BimCatalogueLoadState::Loading {
                received_pages: 0,
                total_pages: 0,
            };
        }
        if let Some(catalogue) = self.bim_classification_catalogue() {
            return BimCatalogueLoadState::Ready(catalogue);
        }
        BimCatalogueLoadState::NotRequested
    }

    /// Registers a newly dispatched `RequestBimProperties` command ID against
    /// the current stage epoch and selection identity.
    pub(crate) fn begin_bim_properties_request(
        &mut self,
        request_id: RequestId,
    ) -> Result<(), &'static str> {
        let Some(snapshot) = &self.snapshot else {
            self.bim_properties = None;
            self.pending_bim = None;
            self.bim_properties_assembly = None;
            self.bim_properties_error = Some("Stage is not loaded".to_owned());
            return Err("Stage is not loaded");
        };
        if !snapshot.stage.loaded {
            self.bim_properties = None;
            self.pending_bim = None;
            self.bim_properties_assembly = None;
            self.bim_properties_error = Some("Stage is not loaded".to_owned());
            return Err("Stage is not loaded");
        }
        if snapshot.selection.targets.is_empty() {
            self.bim_properties = None;
            self.pending_bim = None;
            self.bim_properties_assembly = None;
            self.bim_properties_error = None;
            return Err("Select a prim to read BIM properties");
        }
        self.pending_bim = Some(PendingBimRequest {
            request_id,
            stage_epoch: self.stage_epoch,
            activation_generation: snapshot.stage.activation_generation,
            selection_revision: snapshot.selection_revision,
            targets: snapshot.selection.targets.clone(),
        });
        self.bim_properties = None;
        self.bim_properties_assembly = None;
        self.bim_properties_error = None;
        Ok(())
    }

    /// Registers a newly dispatched `RequestBimClassificationFieldCatalogue`
    /// command ID against the current stage epoch.
    pub(crate) fn begin_bim_catalogue_request(
        &mut self,
        request_id: RequestId,
    ) -> Result<(), &'static str> {
        let Some(snapshot) = &self.snapshot else {
            self.bim_classification_catalogue = None;
            self.pending_catalogue = None;
            self.bim_catalogue_assembly = None;
            self.bim_catalogue_error = Some("Stage is not loaded".to_owned());
            self.catalogue_refresh_needed = None;
            return Err("Stage is not loaded");
        };
        if !snapshot.stage.loaded {
            self.bim_classification_catalogue = None;
            self.pending_catalogue = None;
            self.bim_catalogue_assembly = None;
            self.bim_catalogue_error = Some("Stage is not loaded".to_owned());
            self.catalogue_refresh_needed = None;
            return Err("Stage is not loaded");
        }
        self.pending_catalogue = Some(PendingCatalogueRequest {
            request_id,
            stage_epoch: self.stage_epoch,
            activation_generation: snapshot.stage.activation_generation,
        });
        self.bim_classification_catalogue = None;
        self.bim_catalogue_assembly = None;
        self.bim_catalogue_error = None;
        self.catalogue_refresh_needed = None;
        Ok(())
    }

    pub(crate) fn pending_catalogue_refresh(&self) -> Option<(u64, u64)> {
        self.catalogue_refresh_needed
    }

    /// Consumes a coalesced anonymous-catalogue refresh marker if it matches
    /// the current loaded `(stage_epoch, activation_generation)` and no request
    /// is currently pending.
    pub(crate) fn take_catalogue_refresh_for_current_stage(&mut self) -> bool {
        let Some(marker) = self.catalogue_refresh_needed else {
            return false;
        };
        let Some(snapshot) = &self.snapshot else {
            self.catalogue_refresh_needed = None;
            return false;
        };
        if !snapshot.stage.loaded || self.pending_catalogue.is_some() {
            return false;
        }
        let current = (self.stage_epoch, snapshot.stage.activation_generation);
        if marker != current {
            self.catalogue_refresh_needed = None;
            return false;
        }
        self.catalogue_refresh_needed = None;
        true
    }

    pub(crate) fn clear_catalogue_refresh_needed(&mut self) {
        self.catalogue_refresh_needed = None;
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn clear_last_error(&mut self) {
        self.last_error = None;
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
            ViewportEvent::BimPropertiesRead { properties, .. } => {
                self.apply_bim_properties_read(envelope.request_id.as_deref(), properties);
            }
            ViewportEvent::BimPropertiesPage { page } => {
                self.apply_bim_properties_page(envelope.request_id.as_deref(), page);
            }
            ViewportEvent::BimPropertiesError { error } => {
                self.apply_bim_properties_error(envelope.request_id.as_deref(), error);
            }
            ViewportEvent::BimClassificationFieldCatalogueChanged { catalogue } => {
                self.apply_bim_catalogue_changed(envelope.request_id.as_deref(), catalogue);
            }
            ViewportEvent::BimClassificationFieldCataloguePage { page } => {
                self.apply_bim_catalogue_page(envelope.request_id.as_deref(), page);
            }
            ViewportEvent::CommandRejected { request_id, reason } => {
                self.apply_command_rejected(envelope.request_id.as_deref(), request_id, reason);
            }
            ViewportEvent::HierarchyChildren { .. }
            | ViewportEvent::HierarchySearchResults { .. }
            | ViewportEvent::BimSearchResults { .. }
            | ViewportEvent::BimPropertyProvenanceRead { .. }
            | ViewportEvent::Ready { .. }
            | ViewportEvent::CameraTransitionStarted { .. }
            | ViewportEvent::HierarchyVisibilityChanged { .. } => {}
            ViewportEvent::StageLoadStateChanged { state } => {
                let is_ready = matches!(state, StageLoadState::Ready);
                if !is_ready {
                    self.invalidate_stage();
                    self.clear_scene();
                }
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.stage.loaded = is_ready;
                }
            }
            ViewportEvent::SelectionChanged { selection } => {
                let changed = self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.selection != *selection);
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.selection = selection.clone();
                }
                if changed {
                    self.invalidate_selection();
                }
            }
            ViewportEvent::SelectionDeltaApplied {
                revision,
                added,
                removed,
                primary,
                count,
            } => {
                let mut applied = false;
                if let Some(snapshot) = &mut self.snapshot {
                    let no_op = *revision == snapshot.selection_revision
                        && added.is_empty()
                        && removed.is_empty()
                        && primary.as_ref() == snapshot.selection.primary.as_ref()
                        && *count == snapshot.selection.targets.len() as u32;
                    if no_op {
                        return;
                    }
                    if snapshot.selection_revision.saturating_add(1) == *revision
                        && snapshot
                            .selection
                            .apply_delta(added, removed, primary.clone(), *count)
                            .is_ok()
                    {
                        snapshot.selection_revision = *revision;
                        applied = true;
                    }
                }
                if applied {
                    self.invalidate_selection();
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
            | ViewportEvent::RuntimeMutationBatchAccepted { state, .. }
            | ViewportEvent::BimPropertyEditCompleted { state, .. }
            | ViewportEvent::BimPropertyBatchEditCompleted { state, .. } => {
                self.editor = state.clone();
            }
            ViewportEvent::EditorPrimState { .. }
            | ViewportEvent::EditorStageExportChunk { .. } => {}
        }
    }

    fn apply_snapshot(&mut self, snapshot: ViewportReadModel) {
        if !snapshot.stage.loaded {
            let should_invalidate = self.snapshot.as_ref().is_some_and(|prev| {
                prev.stage.loaded
                    || prev.stage.activation_generation != snapshot.stage.activation_generation
                    || prev.stage.display_name != snapshot.stage.display_name
            });
            if should_invalidate {
                self.invalidate_stage();
            }
            self.clear_scene();
            self.snapshot = Some(snapshot);
            return;
        }

        let stage_changed = self.snapshot.as_ref().is_some_and(|prev| {
            prev.stage.loaded
                && (prev.stage.activation_generation != snapshot.stage.activation_generation
                    || prev.stage.display_name != snapshot.stage.display_name)
        });
        if stage_changed {
            self.invalidate_stage();
        } else if let Some(prev) = &self.snapshot
            && prev.stage.loaded
            && (prev.selection_revision != snapshot.selection_revision
                || prev.selection.targets != snapshot.selection.targets)
        {
            self.invalidate_selection();
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

    fn invalidate_stage(&mut self) {
        self.stage_epoch = self.stage_epoch.saturating_add(1);
        self.bim_properties = None;
        self.pending_bim = None;
        self.bim_properties_assembly = None;
        self.bim_properties_error = None;
        self.bim_classification_catalogue = None;
        self.pending_catalogue = None;
        self.bim_catalogue_assembly = None;
        self.bim_catalogue_error = None;
        self.catalogue_refresh_needed = None;
    }

    fn invalidate_selection(&mut self) {
        self.bim_properties = None;
        self.pending_bim = None;
        self.bim_properties_assembly = None;
        self.bim_properties_error = None;
    }

    fn is_current_pending_bim(&self, request_id: Option<&str>) -> bool {
        let (Some(pending), Some(snapshot)) = (&self.pending_bim, &self.snapshot) else {
            return false;
        };
        request_id == Some(pending.request_id.as_str())
            && snapshot.stage.loaded
            && pending.stage_epoch == self.stage_epoch
            && pending.activation_generation == snapshot.stage.activation_generation
            && pending.selection_revision == snapshot.selection_revision
            && pending.targets == snapshot.selection.targets
    }

    fn is_current_pending_catalogue(&self, request_id: Option<&str>) -> bool {
        let (Some(pending), Some(snapshot)) = (&self.pending_catalogue, &self.snapshot) else {
            return false;
        };
        request_id == Some(pending.request_id.as_str())
            && snapshot.stage.loaded
            && pending.stage_epoch == self.stage_epoch
            && pending.activation_generation == snapshot.stage.activation_generation
    }

    fn fail_bim_properties_request(&mut self, reason: String) {
        self.bim_properties = None;
        self.bim_properties_assembly = None;
        self.pending_bim = None;
        self.bim_properties_error = Some(reason.clone());
        self.last_error = Some(reason);
    }

    fn fail_bim_catalogue_request(&mut self, reason: String) {
        self.bim_classification_catalogue = None;
        self.bim_catalogue_assembly = None;
        self.pending_catalogue = None;
        self.bim_catalogue_error = Some(reason.clone());
        self.catalogue_refresh_needed = None;
        self.last_error = Some(reason);
    }

    fn apply_bim_properties_read(
        &mut self,
        request_id: Option<&str>,
        properties: &BimPropertiesReadModel,
    ) {
        if !self.is_current_pending_bim(request_id) {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        };
        if !snapshot.stage.loaded {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        }
        if properties.selection_revision != snapshot.selection_revision
            || properties.targets != snapshot.selection.targets
        {
            self.fail_bim_properties_request(
                "BIM properties response selection metadata mismatch".to_owned(),
            );
            return;
        }
        self.bim_properties = Some(properties.clone());
        self.pending_bim = None;
        self.bim_properties_assembly = None;
        self.bim_properties_error = None;
    }

    fn apply_bim_properties_page(
        &mut self,
        request_id: Option<&str>,
        page: &BimPropertiesPage,
    ) {
        if !self.is_current_pending_bim(request_id) {
            return;
        }
        let (Some(pending), Some(snapshot)) = (&self.pending_bim, &self.snapshot) else {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        };
        if !snapshot.stage.loaded {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        }
        if page.page_count == 0
            || page.page_count > MAX_BIM_PROPERTY_PAGES
            || page.page_index >= page.page_count
            || (page.total_properties as usize) > MAX_BIM_PROPERTY_COUNT
            || page.selection_revision != snapshot.selection_revision
        {
            self.fail_bim_properties_request("Invalid BIM properties transport page".to_owned());
            return;
        }
        if !page.targets.is_empty() && page.targets != snapshot.selection.targets {
            self.fail_bim_properties_request(
                "BIM properties page targets do not match current selection".to_owned(),
            );
            return;
        }
        if page.page_index == 0 && page.targets != snapshot.selection.targets {
            self.fail_bim_properties_request(
                "BIM properties page 0 targets do not match current selection".to_owned(),
            );
            return;
        }

        let pending_id = pending.request_id.clone();
        let pending_epoch = pending.stage_epoch;
        let pending_gen = pending.activation_generation;
        let pending_rev = pending.selection_revision;
        let pending_targets = pending.targets.clone();

        let assembly = self.bim_properties_assembly.get_or_insert_with(|| {
            BimPropertiesPageAssembly {
                request_id: pending_id.clone(),
                stage_epoch: pending_epoch,
                activation_generation: pending_gen,
                selection_revision: pending_rev,
                targets: pending_targets.clone(),
                page_count: page.page_count,
                total_properties: page.total_properties,
                pages: HashMap::new(),
            }
        });

        if assembly.request_id != pending_id
            || assembly.stage_epoch != pending_epoch
            || assembly.activation_generation != pending_gen
            || assembly.selection_revision != pending_rev
            || assembly.targets != pending_targets
            || assembly.page_count != page.page_count
            || assembly.total_properties != page.total_properties
        {
            self.fail_bim_properties_request(
                "Inconsistent BIM properties transport page metadata".to_owned(),
            );
            return;
        }

        if let Some(existing) = assembly.pages.get(&page.page_index) {
            if existing == page {
                return;
            }
            self.fail_bim_properties_request(
                "Conflicting duplicate BIM properties transport page".to_owned(),
            );
            return;
        }

        assembly.pages.insert(page.page_index, page.clone());
        if assembly.pages.len() < assembly.page_count as usize {
            return;
        }

        let Some(page0) = assembly.pages.get(&0) else {
            self.fail_bim_properties_request(
                "Missing page 0 in BIM properties transport assembly".to_owned(),
            );
            return;
        };
        if page0.targets != snapshot.selection.targets {
            self.fail_bim_properties_request(
                "BIM properties page 0 targets mismatch".to_owned(),
            );
            return;
        }

        let mut merged_groups: Vec<BimPropertyGroupReadModel> = Vec::new();
        let mut group_indices: HashMap<BimPropertyGroupId, usize> = HashMap::new();
        let mut assembled_count: usize = 0;

        for idx in 0..assembly.page_count {
            let Some(p) = assembly.pages.get(&idx) else {
                self.fail_bim_properties_request(
                    "Incomplete BIM properties transport page sequence".to_owned(),
                );
                return;
            };
            for group in &p.groups {
                let Some(next_total) = assembled_count.checked_add(group.properties.len()) else {
                    self.fail_bim_properties_request(
                        "BIM properties count overflow during assembly".to_owned(),
                    );
                    return;
                };
                if next_total > MAX_BIM_PROPERTY_COUNT {
                    self.fail_bim_properties_request(
                        "BIM properties count exceeds maximum limit".to_owned(),
                    );
                    return;
                }
                assembled_count = next_total;
                if let Some(&existing_idx) = group_indices.get(&group.id) {
                    merged_groups[existing_idx]
                        .properties
                        .extend(group.properties.iter().cloned());
                } else {
                    group_indices.insert(group.id, merged_groups.len());
                    merged_groups.push(group.clone());
                }
            }
        }

        let expected_total = assembly.total_properties;
        let selection_revision = assembly.selection_revision;
        if assembled_count != expected_total as usize {
            self.fail_bim_properties_request(format!(
                "BIM properties transport total mismatch: expected {}, assembled {}",
                expected_total, assembled_count
            ));
            return;
        }

        self.bim_properties = Some(BimPropertiesReadModel {
            targets: snapshot.selection.targets.clone(),
            selection_revision,
            groups: merged_groups,
        });
        self.pending_bim = None;
        self.bim_properties_assembly = None;
        self.bim_properties_error = None;
    }

    fn apply_bim_properties_error(
        &mut self,
        request_id: Option<&str>,
        error: &viewport_protocol::BimPropertiesDeliveryError,
    ) {
        if !self.is_current_pending_bim(request_id) {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        };
        if !snapshot.stage.loaded {
            self.fail_bim_properties_request("Current BIM stage unavailable".to_owned());
            return;
        }
        if error.selection_revision != snapshot.selection_revision {
            self.fail_bim_properties_request(
                "BIM properties delivery error selection revision mismatch".to_owned(),
            );
            return;
        }
        self.fail_bim_properties_request(format!(
            "BIM properties delivery error ({:?}): {} / {} bytes",
            error.kind, error.encoded_bytes, error.max_bytes
        ));
    }

    fn apply_bim_catalogue_changed(
        &mut self,
        request_id: Option<&str>,
        catalogue: &BimClassificationFieldCatalogue,
    ) {
        match request_id {
            Some(req_id) => {
                if !self.is_current_pending_catalogue(Some(req_id)) {
                    return;
                }
                let Some(snapshot) = &self.snapshot else {
                    self.fail_bim_catalogue_request("Current BIM stage unavailable".to_owned());
                    return;
                };
                if !snapshot.stage.loaded {
                    self.fail_bim_catalogue_request("Current BIM stage unavailable".to_owned());
                    return;
                }
                self.bim_classification_catalogue = Some(catalogue.clone());
                self.pending_catalogue = None;
                self.bim_catalogue_assembly = None;
                self.bim_catalogue_error = None;
                self.catalogue_refresh_needed = None;
            }
            None => {
                let Some(snapshot) = &self.snapshot else {
                    return;
                };
                if !snapshot.stage.loaded {
                    return;
                }
                // Anonymous (`request_id: None`) catalogue events carry no originating stage epoch
                // or activation generation. Never install them directly; treat them strictly as a
                // coalesced refresh signal for the active stage when no correlated request is pending.
                if self.pending_catalogue.is_none() {
                    self.bim_classification_catalogue = None;
                    self.bim_catalogue_assembly = None;
                    self.bim_catalogue_error = None;
                    self.catalogue_refresh_needed =
                        Some((self.stage_epoch, snapshot.stage.activation_generation));
                }
            }
        }
    }

    fn apply_bim_catalogue_page(
        &mut self,
        request_id: Option<&str>,
        page: &BimClassificationFieldCataloguePage,
    ) {
        let Some(req_id) = request_id else {
            let Some(snapshot) = &self.snapshot else {
                return;
            };
            if !snapshot.stage.loaded {
                return;
            }
            if self.pending_catalogue.is_none() {
                self.bim_classification_catalogue = None;
                self.bim_catalogue_assembly = None;
                self.bim_catalogue_error = None;
                self.catalogue_refresh_needed =
                    Some((self.stage_epoch, snapshot.stage.activation_generation));
            }
            return;
        };

        if !self.is_current_pending_catalogue(Some(req_id)) {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.fail_bim_catalogue_request("Current BIM stage unavailable".to_owned());
            return;
        };
        if !snapshot.stage.loaded {
            self.fail_bim_catalogue_request("Current BIM stage unavailable".to_owned());
            return;
        }
        if page.page_count == 0
            || page.page_count > MAX_BIM_CLASSIFICATION_PAGES
            || page.page_index >= page.page_count
            || (page.total_fields as usize) > MAX_BIM_CLASSIFICATION_FIELDS
        {
            self.fail_bim_catalogue_request(
                "Invalid BIM classification catalogue transport page".to_owned(),
            );
            return;
        }

        let req_owned = req_id.to_owned();
        let stage_epoch = self.stage_epoch;
        let activation_generation = snapshot.stage.activation_generation;

        let assembly = self.bim_catalogue_assembly.get_or_insert_with(|| {
            BimCataloguePageAssembly {
                request_id: req_owned.clone(),
                stage_epoch,
                activation_generation,
                semantic_revision: page.semantic_revision,
                page_count: page.page_count,
                total_fields: page.total_fields,
                pages: HashMap::new(),
            }
        });

        if assembly.request_id != req_owned
            || assembly.stage_epoch != stage_epoch
            || assembly.activation_generation != activation_generation
            || assembly.semantic_revision != page.semantic_revision
            || assembly.page_count != page.page_count
            || assembly.total_fields != page.total_fields
        {
            self.fail_bim_catalogue_request(
                "Inconsistent BIM classification catalogue page metadata".to_owned(),
            );
            return;
        }

        if let Some(existing) = assembly.pages.get(&page.page_index) {
            if existing == page {
                return;
            }
            self.fail_bim_catalogue_request(
                "Conflicting duplicate BIM classification catalogue page".to_owned(),
            );
            return;
        }

        assembly.pages.insert(page.page_index, page.clone());
        if assembly.pages.len() < assembly.page_count as usize {
            return;
        }

        let mut fields = Vec::new();
        for idx in 0..assembly.page_count {
            let Some(p) = assembly.pages.get(&idx) else {
                self.fail_bim_catalogue_request(
                    "Incomplete BIM classification catalogue page sequence".to_owned(),
                );
                return;
            };
            if fields.len().saturating_add(p.fields.len()) > MAX_BIM_CLASSIFICATION_FIELDS {
                self.fail_bim_catalogue_request(
                    "BIM classification catalogue field count exceeds maximum".to_owned(),
                );
                return;
            }
            fields.extend(p.fields.iter().cloned());
        }

        let expected_total = assembly.total_fields;
        let semantic_revision = assembly.semantic_revision;
        if fields.len() != expected_total as usize {
            self.fail_bim_catalogue_request(format!(
                "BIM classification catalogue count mismatch: expected {}, assembled {}",
                expected_total,
                fields.len()
            ));
            return;
        }

        self.bim_classification_catalogue = Some(BimClassificationFieldCatalogue {
            semantic_revision,
            fields,
        });
        self.pending_catalogue = None;
        self.bim_catalogue_assembly = None;
        self.bim_catalogue_error = None;
        self.catalogue_refresh_needed = None;
    }

    fn apply_command_rejected(
        &mut self,
        envelope_request_id: Option<&str>,
        payload_request_id: &str,
        reason: &str,
    ) {
        let matches_bim = self.pending_bim.as_ref().is_some_and(|pending| {
            pending.request_id == payload_request_id
                || envelope_request_id == Some(pending.request_id.as_str())
        });
        if matches_bim {
            self.fail_bim_properties_request(reason.to_owned());
            return;
        }
        let matches_catalogue = self.pending_catalogue.as_ref().is_some_and(|pending| {
            pending.request_id == payload_request_id
                || envelope_request_id == Some(pending.request_id.as_str())
        });
        if matches_catalogue {
            self.fail_bim_catalogue_request(reason.to_owned());
            return;
        }
        self.last_error = Some(reason.to_owned());
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
#[path = "../../../tests/unit/viewport/api/read_model/tests.rs"]
mod tests;
