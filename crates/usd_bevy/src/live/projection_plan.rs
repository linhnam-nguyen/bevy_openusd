use anyhow::{Result, anyhow, ensure};
use openusd::usd::Stage;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use super::path::{parent_path, validate_prim_path};

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
            entries: vec![ProjectionPlanEntry {
                path: "/".to_string(),
                parent: None,
            }],
        }
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
        self.pending.is_empty()
    }

    /// Access a discovered entry while the plan is still being built.
    pub fn entry(&self, index: usize) -> Option<&ProjectionPlanEntry> {
        self.entries.get(index)
    }

    /// Expand one parent and enqueue its valid children in lexical order.
    pub fn advance_one(&mut self) -> Result<bool> {
        let Some((parent_path, parent_index)) = self.pending.pop_front() else {
            return Ok(true);
        };
        let parent = self.stage.prim(openusd::sdf::path(&parent_path)?);
        let mut children = parent.children()?;
        children.sort_unstable_by(|left, right| left.path().as_str().cmp(right.path().as_str()));
        for child in children {
            if !child.is_active()? || !child.is_defined()? || child.is_abstract()? {
                continue;
            }
            let path = child.path().as_str().to_string();
            let index = self.entries.len();
            self.entries.push(ProjectionPlanEntry {
                path: path.clone(),
                parent: Some(parent_index),
            });
            self.pending.push_back((path, index));
        }
        Ok(self.pending.is_empty())
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
        let _ = stage.prim(openusd::sdf::path(&root)?);
        let mut entries = vec![
            ProjectionPlanEntry {
                path: "/".to_string(),
                parent: None,
            },
            ProjectionPlanEntry {
                path: root.clone(),
                parent: Some(0),
            },
        ];
        let mut pending = VecDeque::from([(root, 1usize)]);
        while let Some((parent_path, parent_index)) = pending.pop_front() {
            let parent = stage.prim(openusd::sdf::path(&parent_path)?);
            let mut children = parent.children()?;
            children
                .sort_unstable_by(|left, right| left.path().as_str().cmp(right.path().as_str()));
            for child in children {
                if !child.is_active()? || !child.is_defined()? || child.is_abstract()? {
                    continue;
                }
                let path = child.path().as_str().to_string();
                let index = entries.len();
                entries.push(ProjectionPlanEntry {
                    path: path.clone(),
                    parent: Some(parent_index),
                });
                pending.push_back((path, index));
            }
        }
        Ok(Self { entries })
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
            let prim = stage.prim(openusd::sdf::path(&path)?);
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
