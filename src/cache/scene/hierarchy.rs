//! Rendering-independent logical hierarchy construction from canonical USD
//! stages and validated Scene caches.
//!
//! Logical rows carry stable occurrence identities (`SceneAnchor`), authored
//! metadata, child completeness (`Known` vs `Unknown`), and optional geometry
//! blob references without requiring ECS entity creation or GPU residency.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Result, ensure};
use openusd::usd::{PrimPredicate, PrimStatus, Stage};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::contract::{
    CachedHierarchyDescendant, CachedHierarchyMetadata, HierarchyChildCount,
    SceneCacheActivation, SceneCacheBlobRef, SceneCacheEntryKind, SceneCacheState,
};
use crate::viewport::rendering::scene::prim_name;
use crate::viewport::stage::{StagePresentationContext, resolve_prim_display_name};

/// One rendering-independent logical hierarchy row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogicalPrimRecord {
    pub(crate) anchor: SceneAnchor,
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) prim_path: String,
    pub(crate) label: String,
    pub(crate) display_name: Option<String>,
    pub(crate) type_name: Option<String>,
    pub(crate) child_count: HierarchyChildCount,
    pub(crate) authored_visible: bool,
    pub(crate) geometry_ref: Option<SceneCacheBlobRef>,
}

impl LogicalPrimRecord {
    pub(crate) fn has_children(&self) -> bool {
        self.child_count.has_children()
    }

    pub(crate) fn to_prim_read_model(
        &self,
        visibility_override: Option<bool>,
    ) -> PrimNodeReadModel {
        PrimNodeReadModel {
            anchor: self.anchor.clone(),
            parent: self.parent.clone(),
            label: self.label.clone(),
            display_name: self.display_name.clone(),
            visible: visibility_override.unwrap_or(self.authored_visible),
            has_children: self.has_children(),
        }
    }
}

/// Qualified ancestry before transparent presentation wrappers are collapsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogicalOccurrence {
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) children_unknown: bool,
    pub(crate) authored_visible: bool,
}

/// The displayed rows and the underlying occurrence forest of one candidate.
/// Only a canonical candidate can prove that a missing occurrence was deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogicalOccurrenceForest {
    pub(crate) occurrences: HashMap<SceneAnchor, LogicalOccurrence>,
    pub(crate) canonical: bool,
}

impl LogicalOccurrenceForest {
    /// A missing occurrence is not deleted when its branch is unresolved or the
    /// candidate is provisional. Match complete path components, not prefixes.
    pub(crate) fn absence_unknown(&self, anchor: &SceneAnchor) -> bool {
        if self.occurrences.contains_key(anchor) {
            return false;
        }
        if *anchor != SceneAnchor::active_session(anchor.prim_path.clone()) {
            return false;
        }
        if !self.canonical {
            return true;
        }
        let mut path = parent_path(&anchor.prim_path);
        while let Some(parent) = path {
            let parent_anchor = SceneAnchor::active_session(parent);
            if let Some(occurrence) = self.occurrences.get(&parent_anchor) {
                // The nearest known ancestor owns the missing direct branch.
                return occurrence.children_unknown;
            }
            path = parent_path(parent);
        }
        false
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogicalHierarchyCandidate {
    pub(crate) records: Vec<LogicalPrimRecord>,
    pub(crate) forest: LogicalOccurrenceForest,
}

/// Validates that logical hierarchy records have unique occurrence IDs, valid
/// parent references, and an acyclic parent forest.
pub(crate) fn validate_logical_hierarchy(records: &[LogicalPrimRecord]) -> Result<()> {
    let mut index_by_anchor = HashMap::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        record
            .anchor
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid logical anchor: {error}"))?;
        ensure!(
            index_by_anchor
                .insert(record.anchor.clone(), index)
                .is_none(),
            "duplicate logical occurrence identity {}",
            record.anchor.prim_path
        );
    }

    for (index, record) in records.iter().enumerate() {
        if let Some(parent) = record.parent.as_ref() {
            let Some(&parent_index) = index_by_anchor.get(parent) else {
                anyhow::bail!(
                    "logical hierarchy node {} references unknown parent {}",
                    record.anchor.prim_path,
                    parent.prim_path
                );
            };
            ensure!(
                parent_index != index,
                "logical hierarchy node {} cannot be its own parent",
                record.anchor.prim_path
            );
        }
    }

    // Detect parent cycles across the forest.
    let mut state = vec![0_u8; records.len()];
    for start in 0..records.len() {
        if state[start] != 0 {
            continue;
        }
        let mut chain = Vec::new();
        let mut current = Some(start);
        while let Some(idx) = current {
            if state[idx] == 1 {
                anyhow::bail!(
                    "logical hierarchy contains a parent cycle at {}",
                    records[idx].anchor.prim_path
                );
            }
            if state[idx] == 2 {
                break;
            }
            state[idx] = 1;
            chain.push(idx);
            current = records[idx]
                .parent
                .as_ref()
                .and_then(|parent| index_by_anchor.get(parent).copied());
        }
        for idx in chain {
            state[idx] = 2;
        }
    }
    Ok(())
}

/// Returns whether every top-level entry has qualified hierarchy metadata.
pub(crate) fn cache_has_populated_hierarchy(activation: &SceneCacheActivation) -> bool {
    !activation.index.entries.is_empty()
        && activation.index.entries.iter().all(|entry| entry.hierarchy.metadata_populated)
}

fn logical_traversal_predicate() -> PrimPredicate {
    PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
    .with_instance_proxies(true)
}

pub(super) fn composed_paths(stage: &Stage) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    stage.traverse(logical_traversal_predicate(), |path| {
        if !path.is_abs_root() {
            paths.push(path.as_str().to_owned());
        }
    })?;
    paths.sort_unstable();
    paths.dedup();
    Ok(paths)
}

fn is_descendant(root: &str, path: &str) -> bool {
    path.strip_prefix(root).is_some_and(|suffix| suffix.starts_with('/'))
}

/// Capture member descendants from the parent Scene's sealed composed stage.
/// The snapshot has no render resources and never opens a child cache.
pub(super) fn collect_composed_member_descendants(
    stage: &Stage,
    paths: &[String],
    root: &str,
) -> Result<Vec<CachedHierarchyDescendant>> {
    let mut descendants = paths.iter()
        .filter(|path| is_descendant(root, path))
        .map(|path| {
            let metadata = super::build::extract_owned_prim_hierarchy(stage, path)?;
            Ok(CachedHierarchyDescendant { prim_path: path.clone(), metadata })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut counts: HashMap<String, u32> = HashMap::new();
    for row in &descendants {
        if let Some(parent) = parent_path(&row.prim_path) {
            let count = counts.entry(parent.to_owned()).or_default();
            *count = count.saturating_add(1);
        }
    }
    for row in &mut descendants {
        if row.metadata.child_count != HierarchyChildCount::Unknown {
            row.metadata.child_count = HierarchyChildCount::Known(counts.get(&row.prim_path).copied().unwrap_or(0));
        }
    }
    Ok(descendants)
}

struct RawNode {
    path: String,
    metadata: CachedHierarchyMetadata,
    geometry_ref: Option<SceneCacheBlobRef>,
}

/// Build canonical logical rows without rendering or creating entities.
pub(crate) fn build_logical_hierarchy_from_stage(
    stage: &Stage,
    presentation: Option<&StagePresentationContext>,
) -> Result<LogicalHierarchyCandidate> {
    let nodes = composed_paths(stage)?
        .into_iter()
        .map(|path| {
            let metadata = super::build::extract_owned_prim_hierarchy(stage, &path)?;
            Ok(RawNode { path, metadata, geometry_ref: None })
        })
        .collect::<Result<Vec<_>>>()?;
    collapse_logical_rows(nodes, presentation, true)
}

/// Build a provisional hierarchy from one qualified Scene revision. Nested
/// rows are captured from this revision's composed member occurrences, rather
/// than copied from independently current child Scene revisions.
pub(crate) fn build_logical_hierarchy_from_cache(
    activation: &SceneCacheActivation,
    _project_root: Option<&Path>,
    presentation: Option<&StagePresentationContext>,
) -> Result<LogicalHierarchyCandidate> {
    activation.index.validate(activation.descriptor.scene_id, activation.descriptor.generation)?;
    ensure!(cache_has_populated_hierarchy(activation), "Scene cache hierarchy is unqualified");
    let incomplete = activation.descriptor.state != SceneCacheState::Ready;
    let mut nodes = Vec::new();
    for entry in &activation.index.entries {
        let (path, member) = match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => (prim_path.clone(), false),
            SceneCacheEntryKind::ChildScene { member_id, .. }
            | SceneCacheEntryKind::ChildModel { member_id, .. } => (
                crate::project::scene::authoring::scene_member_path(*member_id), true,
            ),
        };
        let mut metadata = entry.hierarchy.clone();
        let descendants = metadata.composed_descendants.take();
        ensure!(member || descendants.is_none(), "owned prim has a member snapshot: {path}");
        if member {
            match descendants {
                Some(descendants) => {
                    let mut paths = HashSet::from([path.as_str()]);
                    for row in &descendants {
                        ensure!(is_descendant(&path, &row.prim_path)
                            && row.metadata.metadata_populated
                            && row.metadata.composed_descendants.is_none(),
                            "invalid composed member descendant {}", row.prim_path);
                        ensure!(paths.insert(row.prim_path.as_str()),
                            "duplicate composed member descendant {}", row.prim_path);
                    }
                    for row in &descendants {
                        ensure!(row.prim_path.rsplit_once('/').is_some_and(|(parent, _)| paths.contains(parent)),
                            "missing composed member parent for {}", row.prim_path);
                    }
                    for row in descendants {
                        let mut metadata = row.metadata;
                        if incomplete {
                            metadata.child_count = HierarchyChildCount::Unknown;
                        }
                        nodes.push(RawNode { path: row.prim_path, metadata, geometry_ref: None });
                    }
                }
                None => metadata.child_count = HierarchyChildCount::Unknown,
            }
        }
        if incomplete {
            metadata.child_count = HierarchyChildCount::Unknown;
        }
        nodes.push(RawNode { path, metadata, geometry_ref: entry.geometry.clone() });
    }
    collapse_logical_rows(nodes, presentation, false)
}

fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent).filter(|parent| !parent.is_empty())
}

/// Collapse only qualified, resolved transparent wrappers. An unresolved
/// wrapper remains an expandable row, so its unknown branch cannot disappear.
fn collapse_logical_rows(
    nodes: Vec<RawNode>,
    presentation: Option<&StagePresentationContext>,
    canonical: bool,
) -> Result<LogicalHierarchyCandidate> {
    let mut by_path = HashMap::with_capacity(nodes.len());
    for node in &nodes {
        let anchor = SceneAnchor::active_session(node.path.clone());
        anchor.validate().map_err(|error| anyhow::anyhow!("invalid logical path: {error}"))?;
        ensure!(node.metadata.metadata_populated, "unqualified hierarchy row {}", node.path);
        ensure!(by_path.insert(node.path.as_str(), &node.metadata).is_none(),
            "duplicate composed hierarchy path {}", node.path);
    }
    // Build the underlying parent forest from the same validated raw rows.
    // Parent paths shorten at every step, so cycles are impossible. A missing
    // immediate parent in a cache is not invented as a qualified occurrence.
    let occurrences = nodes.iter().map(|node| {
        let parent = parent_path(&node.path)
            .filter(|path| by_path.contains_key(path))
            .map(SceneAnchor::active_session);
        (SceneAnchor::active_session(node.path.clone()), LogicalOccurrence {
            parent,
            children_unknown: node.metadata.child_count == HierarchyChildCount::Unknown,
            authored_visible: node.metadata.authored_visible,
        })
    }).collect();
    let collapsed = |metadata: &CachedHierarchyMetadata| {
        metadata.transparent && metadata.child_count != HierarchyChildCount::Unknown
    };
    let mut records = Vec::with_capacity(nodes.len());
    for node in &nodes {
        if collapsed(&node.metadata) {
            continue;
        }
        let mut parent = parent_path(&node.path);
        while let Some(path) = parent {
            match by_path.get(path) {
                Some(metadata) if collapsed(metadata) => parent = parent_path(path),
                Some(_) => break,
                None => { parent = None; break; }
            }
        }
        let metadata = &node.metadata;
        let display_name = resolve_prim_display_name(
            presentation, &node.path, metadata.display_name.as_deref(),
            metadata.target_kind.as_deref().zip(metadata.target_id.as_deref()),
        );
        records.push(LogicalPrimRecord {
            anchor: SceneAnchor::active_session(node.path.clone()),
            parent: parent.map(SceneAnchor::active_session),
            label: prim_name(&node.path).to_owned(),
            prim_path: node.path.clone(),
            display_name,
            type_name: metadata.type_name.clone(),
            child_count: metadata.child_count,
            authored_visible: metadata.authored_visible,
            geometry_ref: node.geometry_ref.clone(),
        });
    }
    let index_by_path: HashMap<_, _> = records.iter().enumerate()
        .map(|(index, record)| (record.prim_path.clone(), index)).collect();
    let mut counts = vec![0_u32; records.len()];
    for record in &records {
        if let Some(parent) = record.parent.as_ref()
            && let Some(&index) = index_by_path.get(&parent.prim_path)
        {
            counts[index] = counts[index].saturating_add(1);
        }
    }
    for (record, count) in records.iter_mut().zip(counts) {
        if record.child_count != HierarchyChildCount::Unknown {
            record.child_count = HierarchyChildCount::Known(count);
        }
    }
    records.sort_unstable_by(|left, right| left.anchor.cmp(&right.anchor));
    validate_logical_hierarchy(&records)?;
    Ok(LogicalHierarchyCandidate {
        records,
        forest: LogicalOccurrenceForest { occurrences, canonical },
    })
}
