use anyhow::{Result, anyhow};
use openusd::usd::Stage;
use std::collections::HashMap;

use super::path::{parent_path, validate_prim_path};
use super::projection::traverse_predicate;

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

impl ProjectionPlan {
    /// Build a plan using the same active/defined/non-abstract predicate as
    /// ordinary live projection and subtree reconciliation.
    pub fn from_stage(stage: &Stage) -> Result<Self> {
        let mut paths = vec!["/".to_string()];
        stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
            if path.as_str() != "/" {
                paths.push(path.as_str().to_string());
            }
        })?;
        paths.sort_unstable_by(|left, right| {
            left.matches('/')
                .count()
                .cmp(&right.matches('/').count())
                .then_with(|| left.cmp(right))
        });

        let indices: HashMap<String, usize> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| (path.clone(), index))
            .collect();
        let mut entries = Vec::with_capacity(paths.len());
        for path in paths {
            let parent = if path == "/" {
                None
            } else {
                let parent_path = parent_path(&path);
                Some(*indices.get(parent_path).ok_or_else(|| {
                    anyhow!("projection parent '{parent_path}' missing for '{path}'")
                })?)
            };
            entries.push(ProjectionPlanEntry { path, parent });
        }
        Ok(Self { entries })
    }

    /// Build a plan and validate its root argument before filtering it.
    pub fn from_subtree(stage: &Stage, root: &str) -> Result<Self> {
        let root = validate_prim_path(root)?;
        let full = Self::from_stage(stage)?;
        let mut entries = Vec::new();
        let mut remap = HashMap::new();
        for entry in &full.entries {
            if entry.path == "/"
                || entry.path == root
                || entry.path.starts_with(&format!("{root}/"))
            {
                let index = entries.len();
                remap.insert(entry.path.clone(), index);
                entries.push(entry.clone());
            }
        }
        for entry in &mut entries {
            entry.parent = if entry.path == "/" {
                None
            } else {
                remap.get(parent_path(&entry.path)).copied()
            };
        }
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
