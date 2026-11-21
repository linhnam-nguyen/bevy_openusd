use anyhow::{Result, anyhow, ensure};
use openusd::usd::{Prim, PrimPredicate, PrimStatus, Stage};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use super::path::{parent_path, validate_prim_path};
use crate::read::geom::{VisibilityState, read_visibility};

// Retain unloaded payload roots so logical projection can publish placeholders.
fn projection_predicate() -> PrimPredicate {
    PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
    .with_instance_proxies(true)
}

/// One deterministic unit of initial projection work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPlanEntry {
    path: String,
    parent: Option<usize>,
}

impl ProjectionPlanEntry {
    /// The composed absolute prim path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The parent entry index, or `None` for the synthetic stage root.
    pub fn parent_index(&self) -> Option<usize> {
        self.parent
    }
}

/// A stable parent-before-child projection order for one composed stage.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectionPlan {
    entries: Vec<ProjectionPlanEntry>,
}

/// Incrementally builds a deterministic parent-before-child plan.
#[derive(Clone)]
pub struct ProjectionPlanBuilder {
    stage: Stage,
    pending: VecDeque<(String, usize)>,
    deferred: VecDeque<(String, usize)>,
    children: Option<(usize, std::vec::IntoIter<Prim>)>,
    prioritize_visibility: bool,
    entries: Vec<ProjectionPlanEntry>,
}

impl fmt::Debug for ProjectionPlanBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionPlanBuilder")
            .field("pending", &self.pending)
            .field("entries", &self.entries)
            .finish()
    }
}

impl ProjectionPlanBuilder {
    /// Start with only the synthetic stage root; no scene traversal occurs.
    pub fn new(stage: &Stage) -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(("/".to_string(), 0));
        Self {
            stage: stage.clone(),
            pending,
            deferred: VecDeque::new(),
            children: None,
            prioritize_visibility: false,
            entries: vec![ProjectionPlanEntry {
                path: "/".to_string(),
                parent: None,
            }],
        }
    }

    pub(super) fn visible_first(stage: &Stage) -> Self {
        let mut builder = Self::new(stage);
        builder.prioritize_visibility = true;
        builder
    }

    pub(super) fn visible_discovery_finished(&self) -> bool {
        self.prioritize_visibility && self.pending.is_empty() && self.children.is_none()
    }

    /// Number of entries discovered so far, including the synthetic root.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no projection entries have been discovered yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether every queued parent has been expanded.
    pub fn is_finished(&self) -> bool {
        self.pending.is_empty() && self.deferred.is_empty() && self.children.is_none()
    }

    /// Access a discovered entry while the plan is still being built.
    pub fn entry(&self, index: usize) -> Option<&ProjectionPlanEntry> {
        self.entries.get(index)
    }

    /// Expand one parent, classify one visible-phase child, or admit one deferred subtree.
    pub fn advance_one(&mut self) -> Result<bool> {
        if let Some((parent, children)) = &mut self.children {
            let parent = *parent;
            let child = children.next().expect("pending children is nonempty");
            if children.len() == 0 {
                self.children = None;
            }
            let path = child.path().as_str().to_string();
            if read_visibility(&self.stage, child.path())? == VisibilityState::Invisible {
                // Invisibility is inherited, so the whole subtree can wait.
                self.deferred.push_back((path, parent));
            } else {
                self.enqueue(path, parent);
            }
            return Ok(self.is_finished());
        }
        let Some((parent_path, parent_index)) = self.pending.pop_front() else {
            if let Some((path, parent)) = self.deferred.pop_front() {
                self.prioritize_visibility = false;
                self.enqueue(path, parent);
            }
            return Ok(self.is_finished());
        };
        let parent = self.stage.prim(openusd::sdf::path(&parent_path)?)?;
        let mut children = parent.filtered_children(projection_predicate())?;
        children.sort_unstable_by(|left, right| left.path().as_str().cmp(right.path().as_str()));
        if self.prioritize_visibility && !children.is_empty() {
            self.children = Some((parent_index, children.into_iter()));
        } else {
            for child in children {
                self.enqueue(child.path().as_str().to_string(), parent_index);
            }
        }
        Ok(self.is_finished())
    }

    fn enqueue(&mut self, path: String, parent: usize) {
        let index = self.entries.len();
        self.entries.push(ProjectionPlanEntry {
            path: path.clone(),
            parent: Some(parent),
        });
        self.pending.push_back((path, index));
    }

    /// Finish an exhausted builder and return its immutable plan.
    pub fn finish(self) -> Result<ProjectionPlan> {
        if !self.is_finished() {
            return Err(anyhow!("projection plan builder still has pending parents"));
        }
        Ok(ProjectionPlan {
            entries: self.entries,
        })
    }
}

impl ProjectionPlan {
    /// Build a plan using the same proxy-aware active/defined/non-abstract
    /// predicate as ordinary live projection and subtree reconciliation.
    pub fn from_stage(stage: &Stage) -> Result<Self> {
        let mut builder = ProjectionPlanBuilder::new(stage);
        while !builder.is_finished() {
            builder.advance_one()?;
        }
        builder.finish()
    }

    /// Build a plan and validate its root argument before filtering it.
    pub fn from_subtree(stage: &Stage, root: &str) -> Result<Self> {
        let root = validate_prim_path(root)?;
        if root == "/" {
            return Self::from_stage(stage);
        }

        // Reconcile only the requested namespace. The previous implementation
        // walked the entire composed stage and filtered afterward, which made
        // a small subtree resync pay the full-stage traversal cost.
        let _ = stage.prim(openusd::sdf::path(&root)?)?;
        let mut builder = ProjectionPlanBuilder::new(stage);
        builder.pending.clear();
        builder.enqueue(root, 0);
        while !builder.is_finished() {
            builder.advance_one()?;
        }
        builder.finish()
    }

    /// Build a parent-before-child plan for exact requested prim paths.
    ///
    /// Unlike [`Self::from_stage`] and [`Self::from_subtree`], this never
    /// traverses children. Namespace ancestors are included only so the
    /// requested prim can be projected into the existing ECS hierarchy.
    pub fn from_paths(stage: &Stage, requested_paths: &[&str]) -> Result<Self> {
        let mut requested = BTreeSet::new();
        for raw_path in requested_paths {
            let path = validate_prim_path(raw_path)?;
            ensure!(path != "/", "targeted projection cannot request stage root");
            let prim = stage.prim(openusd::sdf::path(&path)?)?;
            ensure!(prim.is_active()?, "targeted prim {path} is inactive");
            ensure!(prim.is_defined()?, "targeted prim {path} is undefined");
            ensure!(!prim.is_abstract()?, "targeted prim {path} is abstract");
            requested.insert(path);
        }

        let mut paths = BTreeSet::from(["/".to_string()]);
        for path in requested {
            let mut current = path.as_str();
            loop {
                paths.insert(current.to_string());
                if current == "/" {
                    break;
                }
                current = parent_path(current);
            }
        }

        let mut ordered = paths.into_iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            let left_depth = left
                .split('/')
                .filter(|segment| !segment.is_empty())
                .count();
            let right_depth = right
                .split('/')
                .filter(|segment| !segment.is_empty())
                .count();
            left_depth.cmp(&right_depth).then_with(|| left.cmp(right))
        });
        let indexes = ordered
            .iter()
            .enumerate()
            .map(|(index, path)| (path.clone(), index))
            .collect::<HashMap<_, _>>();
        let entries = ordered
            .into_iter()
            .map(|path| ProjectionPlanEntry {
                parent: (path != "/").then(|| indexes[parent_path(&path)]),
                path,
            })
            .collect();
        Ok(Self { entries })
    }

    /// Number of synthetic-root plus composed-prim work items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the plan contains no work items.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access one work item by deterministic index.
    pub fn entry(&self, index: usize) -> Option<&ProjectionPlanEntry> {
        self.entries.get(index)
    }

    /// Iterate work items in parent-before-child order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &ProjectionPlanEntry> {
        self.entries.iter()
    }

    /// Iterate only paths, preserving the plan order.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = &str> {
        self.entries.iter().map(|entry| entry.path())
    }
}
