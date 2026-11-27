use std::collections::HashMap;

use bevy::prelude::Entity;
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_PAGE_SIZE, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::{DenseSceneIndex, SceneAnchorIndex};
use crate::cache::scene::hierarchy::LogicalHierarchyCandidate;

impl SceneAnchorIndex {
    pub(crate) fn has_logical_authority(&self) -> bool {
        self.logical_authority
    }

    pub(crate) fn logical_hierarchy_available(&self) -> bool {
        self.logical_authority && self.logical_available
    }

    fn clear_logical_projection(&mut self) {
        if self.logical_authority {
            self.nodes.clear();
            self.node_index_by_anchor.clear();
            self.dense = DenseSceneIndex::default();
            self.by_anchor.clear();
            self.by_entity.clear();
            self.occurrence_index = Default::default();
            self.parent_by_entity.clear();
            self.transparent_by_entity.clear();
            self.visual_unload_rows.clear();
            self.reconcile = Default::default();
        }
        self.logical_available = false;
        self.canonical_hierarchy_stamp = None;
        self.logical_records.clear();
        self.logical_index_by_anchor.clear();
        self.logical_occurrences = Default::default();
        self.visibility_overrides.clear();
    }

    /// Clear the previous activation, including transient intent.
    pub(crate) fn clear_logical_hierarchy(&mut self) {
        self.clear_logical_projection();
        self.logical_authority = false;
        self.visibility_scopes.clear();
        self.visibility_sequence = 0;
    }

    /// Failed extraction is not evidence of deletion. Stop authorizing targets
    /// from this projection but keep same-activation scope intent pending.
    pub(crate) fn mark_logical_hierarchy_unavailable(&mut self, generation: u64) {
        if self.active_activation_generation != Some(generation) {
            self.clear_logical_hierarchy();
            self.active_activation_generation = Some(generation);
        }
        if !self.logical_authority || self.logical_available {
            // Legacy mappings must not authorize stale targets either.
            self.logical_authority = true;
            self.clear_logical_projection();
            self.published_revision = self.published_revision.saturating_add(1);
        }
        self.published_activation_generation = None;
    }

    pub(super) fn logical_candidate_changed(&self, candidate: &LogicalHierarchyCandidate) -> bool {
        !self.logical_hierarchy_available()
            || self.logical_records != candidate.records
            || self.logical_occurrences != candidate.forest
    }

    /// Selection may remain pending while a branch is unresolved. A collapsed
    /// known wrapper is not a displayed selection target; its visibility scope
    /// is retained separately. No pending target authorizes a new command.
    pub(crate) fn retains_logical_selection(&self, anchor: &SceneAnchor) -> bool {
        self.contains_logical_anchor(anchor)
            || (self.logical_authority && (!self.logical_available
                || self.logical_occurrences.absence_unknown(anchor)))
    }

    /// Replace logical authority, including a valid empty namespace. Only
    /// surviving identities in the same activation retain transient intent.
    pub(crate) fn install_logical_hierarchy(
        &mut self,
        candidate: LogicalHierarchyCandidate,
        activation_generation: u64,
    ) -> CurrentHierarchyProjection {
        if self.active_activation_generation != Some(activation_generation) {
            self.clear_logical_hierarchy();
            self.active_activation_generation = Some(activation_generation);
        }
        if !self.logical_candidate_changed(&candidate) {
            return self.prim_projection();
        }
        let LogicalHierarchyCandidate { records, forest } = candidate;
        let logical_index_by_anchor: HashMap<SceneAnchor, usize> = records
            .iter().enumerate()
            .map(|(index, record)| (record.anchor.clone(), index)).collect();
        self.visibility_scopes.retain(|anchor, _| {
            forest.occurrences.contains_key(anchor) || forest.absence_unknown(anchor)
        });
        // Prune obsolete resident mappings; an old ECS entity cannot authorize
        // a deleted logical command target. Transparent support entities can
        // be rebound independently when the renderer admits them.
        let obsolete: Vec<_> = self.by_entity.iter()
            .filter(|(_, anchor)| !forest.occurrences.contains_key(*anchor))
            .map(|(&entity, _)| entity).collect();
        for entity in obsolete {
            self.evict_resident_entity(entity);
        }
        self.logical_records = records;
        self.logical_index_by_anchor = logical_index_by_anchor;
        self.logical_authority = true;
        self.logical_available = true;
        self.logical_occurrences = forest;
        self.recompute_visibility_overrides();
        self.nodes = self.logical_records.iter().map(|record| {
            record.to_prim_read_model(self.visibility_overrides.get(&record.anchor).copied())
        }).collect();
        self.node_index_by_anchor = self.nodes.iter().enumerate()
            .map(|(index, node)| (node.anchor.clone(), index)).collect();
        self.visual_unload_rows.clear();
        self.dense = DenseSceneIndex::from_nodes(&self.nodes, &self.by_anchor);
        self.published_revision = self.published_revision.saturating_add(1);
        self.published_activation_generation = Some(activation_generation);
        self.initialized = true;
        self.derived_dirty = false;
        self.reconcile = Default::default();
        self.prim_projection()
    }

    /// Path-only semantic targets can address qualified collapsed support prims,
    /// but never a missing occurrence or a failed candidate.
    pub(crate) fn accepts_logical_prim_path(&self, path: &str) -> bool {
        !self.logical_authority || (self.logical_available
            && self.logical_occurrences.occurrences.contains_key(&SceneAnchor::active_session(path)))
    }

    pub(crate) fn contains_logical_anchor(&self, anchor: &SceneAnchor) -> bool {
        if self.logical_authority {
            self.logical_available && (self.logical_index_by_anchor.contains_key(anchor)
                || self.is_point_target(anchor))
        } else {
            self.dense.by_anchor.contains_key(anchor) || self.by_anchor.contains_key(anchor)
        }
    }

    pub(crate) fn visibility_override(&self, anchor: &SceneAnchor) -> Option<bool> {
        self.visibility_overrides.get(anchor).copied()
    }

    /// Effective visibility for a qualified occurrence, including authored state.
    pub(crate) fn logical_visibility(&self, anchor: &SceneAnchor) -> Option<bool> {
        let occurrence = self.logical_occurrences.occurrences.get(anchor)?;
        Some(self.visibility_override(anchor).unwrap_or(occurrence.authored_visible))
    }

    fn recompute_visibility_overrides(&mut self) {
        self.visibility_overrides.clear();
        // The parent forest is validated by the logical builder. Memoize each
        // ancestor result so a deep hierarchy is processed once per row.
        let mut effective: HashMap<SceneAnchor, Option<(u64, bool)>> = HashMap::new();
        let occurrences = &self.logical_occurrences.occurrences;
        for root in occurrences.keys() {
            let mut chain = Vec::new();
            let mut next = Some(root);
            while let Some(anchor) = next {
                if effective.contains_key(anchor) || chain.len() >= occurrences.len() {
                    break;
                }
                let Some(occurrence) = occurrences.get(anchor) else { break; };
                chain.push(anchor.clone());
                next = occurrence.parent.as_ref();
            }
            let mut inherited = next.and_then(|anchor| effective.get(anchor).copied().flatten());
            for anchor in chain.into_iter().rev() {
                if let Some(&scope) = self.visibility_scopes.get(&anchor)
                    && inherited.is_none_or(|previous| scope.0 >= previous.0)
                {
                    inherited = Some(scope);
                }
                effective.insert(anchor, inherited);
            }
        }
        self.visibility_overrides = effective.into_iter()
            .filter_map(|(anchor, scope)| scope.map(|(_, visible)| (anchor, visible)))
            .collect();
    }

    /// Record a subtree command even when some descendants are not yet known.
    /// Overlapping commands preserve the existing last-command-wins behavior.
    pub(crate) fn set_logical_subtree_visibility(
        &mut self,
        root: &SceneAnchor,
        visible: bool,
    ) -> bool {
        if !self.contains_logical_anchor(root) {
            return false;
        }
        if self.logical_authority && self.logical_index_by_anchor.contains_key(root) {
            self.record_visibility_scope(root, visible);
            self.refresh_scoped_visibility();
        } else {
            // Legacy/point-instance targets retain their established behavior.
            let mut affected = self.dense.subtree_anchors(root);
            if affected.is_empty() { affected.push(root.clone()); }
            for anchor in affected {
                self.visibility_overrides.insert(anchor.clone(), visible);
                self.dense.set_visibility(&anchor, visible);
                if let Some(&index) = self.node_index_by_anchor.get(&anchor)
                    && let Some(node) = self.nodes.get_mut(index)
                {
                    node.visible = visible;
                }
            }
        }
        true
    }

    fn record_visibility_scope(&mut self, root: &SceneAnchor, visible: bool) {
        self.visibility_sequence = self.visibility_sequence.saturating_add(1);
        self.visibility_scopes.insert(root.clone(), (self.visibility_sequence, visible));
    }

    fn refresh_scoped_visibility(&mut self) {
        self.recompute_visibility_overrides();
        for record in &self.logical_records {
            let effective = self.visibility_overrides.get(&record.anchor)
                .copied().unwrap_or(record.authored_visible);
            self.dense.set_visibility(&record.anchor, effective);
            if let Some(&index) = self.node_index_by_anchor.get(&record.anchor)
                && let Some(node) = self.nodes.get_mut(index)
            {
                node.visible = effective;
            }
        }
    }

    pub(crate) fn set_logical_prim_paths_visibility(&mut self, paths: &[String], visible: bool) {
        if self.logical_authority {
            if !self.logical_available {
                return;
            }
            for path in paths {
                let anchor = SceneAnchor::active_session(path.clone());
                if self.logical_occurrences.occurrences.contains_key(&anchor) {
                    self.record_visibility_scope(&anchor, visible);
                }
            }
            self.refresh_scoped_visibility();
            return;
        }
        let anchors: Vec<SceneAnchor> = paths.iter()
            .flat_map(|path| self.dense.anchors_for_prim_path(path)).collect();
        for anchor in anchors { self.set_logical_subtree_visibility(&anchor, visible); }
    }

    /// Remove residency, never the canonical namespace or its scoped intents.
    pub(crate) fn evict_resident_entity(&mut self, entity: Entity) {
        self.parent_by_entity.remove(&entity);
        self.transparent_by_entity.remove(&entity);
        self.dense.unbind_entity(entity);
        if let Some(anchor) = self.by_entity.remove(&entity) {
            self.occurrence_index.remove(&anchor.prim_path, entity);
            if self.by_anchor.get(&anchor) == Some(&entity) {
                self.by_anchor.remove(&anchor);
                if let Some(replacement) = self.occurrence_index.resolve(&anchor.prim_path)
                    .iter().copied().find(|candidate| self.by_entity.get(candidate) == Some(&anchor))
                {
                    self.by_anchor.insert(anchor.clone(), replacement);
                    self.dense.bind_entity(&anchor, replacement);
                }
            }
        }
    }

    pub(super) fn bind_resident_entities_to_logical(
        &mut self,
        entities: impl IntoIterator<Item = (Entity, String, bool)>,
    ) -> (Vec<(Entity, bool)>, Vec<Entity>) {
        let mut pending_visibility = Vec::new();
        let mut unmapped = Vec::new();
        for (entity, prim_path, transparent) in entities {
            let anchor = SceneAnchor::active_session(prim_path);
            if !self.logical_occurrences.occurrences.contains_key(&anchor) {
                self.evict_resident_entity(entity);
                unmapped.push(entity);
                continue;
            }
            if self.by_entity.get(&entity).is_some_and(|previous| previous != &anchor) {
                self.evict_resident_entity(entity);
            }
            let representative = *self.by_anchor.entry(anchor.clone()).or_insert(entity);
            if self.by_entity.insert(entity, anchor.clone()).is_none() {
                self.occurrence_index.insert(&anchor.prim_path, entity);
            }
            self.transparent_by_entity.insert(entity, transparent);
            self.dense.bind_entity(&anchor, representative);
            if let Some(visible) = self.visibility_override(&anchor) {
                pending_visibility.push((entity, visible));
            }
        }
        (pending_visibility, unmapped)
    }

    pub(crate) fn resident_visibility_intents(&self) -> Vec<(Entity, bool)> {
        self.by_entity.iter().filter_map(|(&entity, anchor)| {
            self.visibility_override(anchor).map(|visible| (entity, visible))
        }).collect()
    }

    pub(crate) fn prim_projection(&self) -> CurrentHierarchyProjection {
        if self.logical_authority && !self.logical_available {
            return CurrentHierarchyProjection::empty(viewport_protocol::HierarchySource::Prim, 0);
        }
        if self.logical_authority || self.visual_unload_rows.is_empty() {
            return CurrentHierarchyProjection::from_prim_nodes(
                &self.nodes,
                self.published_revision,
            );
        }
        CurrentHierarchyProjection::from_prim_nodes(&self.logical_nodes(), self.published_revision)
    }

    pub(crate) fn has_published_nodes(&self) -> bool {
        !self.dense.nodes.is_empty()
    }

    /// Snapshot logical rows before an automatic visual-only payload unload.
    /// Scene and selection resolution still use live entities when available.
    pub(crate) fn retain_visual_unload_rows(&mut self, roots: &[String]) {
        for node in &self.nodes {
            if roots.iter().any(|root| {
                node.anchor
                    .prim_path
                    .strip_prefix(root)
                    .is_some_and(|tail| tail.starts_with('/'))
            }) {
                self.visual_unload_rows
                    .insert(node.anchor.clone(), node.clone());
            }
        }
    }

    pub(crate) fn clear_visual_unload_rows(&mut self) {
        if !self.visual_unload_rows.is_empty() {
            self.visual_unload_rows.clear();
            self.derived_dirty = true;
        }
    }

    fn logical_nodes(&self) -> Vec<viewport_protocol::PrimNodeReadModel> {
        let mut by_anchor: std::collections::HashMap<_, _> = self
            .visual_unload_rows
            .values()
            .map(|row| (row.anchor.clone(), row.clone()))
            .collect();
        for node in &self.nodes {
            by_anchor.insert(node.anchor.clone(), node.clone());
        }
        for row in by_anchor.values_mut() {
            row.has_children = false;
        }
        let parents: Vec<_> = by_anchor
            .values()
            .filter_map(|row| row.parent.clone())
            .collect();
        for parent in parents {
            if let Some(row) = by_anchor.get_mut(&parent) {
                row.has_children = true;
            }
        }
        let mut rows: Vec<_> = by_anchor.into_values().collect();
        rows.sort_unstable_by(|a, b| a.anchor.cmp(&b.anchor));
        rows
    }

    pub(super) fn rebuild_logical_dense(&mut self) {
        if self.visual_unload_rows.is_empty() {
            self.dense = DenseSceneIndex::from_nodes(&self.nodes, &self.by_anchor);
        } else {
            self.dense = DenseSceneIndex::from_nodes(&self.logical_nodes(), &self.by_anchor);
        }
    }

    /// Returns the bounded initial tree payload. Descendants stay in the
    /// authoritative server index and are requested by the client when a
    /// parent is expanded.
    pub(crate) fn roots_read_model(&self) -> SceneReadModel {
        let page = self.children_page(None, 0, DEFAULT_SCENE_PAGE_SIZE);
        SceneReadModel {
            prims: page.nodes,
            total_prims: self.dense.nodes.len() as u32,
            total_roots: page.total,
            root_page_size: page.page_size,
        }
    }

    pub(crate) fn children_page(
        &self,
        parent: Option<&SceneAnchor>,
        page: u32,
        page_size: u32,
    ) -> SceneChildrenPage {
        let page_size = if page_size == 0 {
            DEFAULT_SCENE_PAGE_SIZE
        } else {
            page_size.min(MAX_SCENE_PAGE_SIZE)
        };
        let children = match parent {
            Some(parent) => self
                .dense
                .by_anchor
                .get(parent)
                .map(|index| self.dense.children(Some(*index)))
                .unwrap_or_default(),
            None => self.dense.children(None),
        };
        let total = children.len() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let page_nodes = children
            .into_iter()
            .skip(start)
            .take(page_size as usize)
            .filter_map(|index| self.dense.protocol_node(*index))
            .collect();

        SceneChildrenPage {
            parent: parent.cloned(),
            page,
            page_size,
            total,
            nodes: page_nodes,
        }
    }

    pub(crate) fn search_match_for_path(&self, prim_path: &str) -> Option<SceneSearchMatch> {
        let node_index = self.dense.by_path.get(prim_path)?.first().copied()?;
        let node = self.dense.node(node_index)?;
        let mut ancestry = Vec::new();
        let mut current = Some(node_index);
        while let Some(index) = current {
            let node = self.dense.node(index)?;
            ancestry.push(index);
            current = node.parent;
        }

        let reveal_pages = ancestry
            .into_iter()
            .rev()
            .filter_map(|index| {
                let node = self.dense.node(index)?;
                Some(ScenePageReference {
                    parent: node
                        .parent
                        .and_then(|parent| self.dense.node(parent))
                        .map(|node| node.anchor.clone()),
                    page: (node.sibling_index as u32) / DEFAULT_SCENE_PAGE_SIZE,
                })
            })
            .collect();

        Some(SceneSearchMatch {
            anchor: node.anchor.clone(),
            parent: node
                .parent
                .and_then(|parent| self.dense.node(parent))
                .map(|node| node.anchor.clone()),
            label: node.label.clone(),
            breadcrumb: node.anchor.prim_path.clone(),
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages,
        })
    }
}
