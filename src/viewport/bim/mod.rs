//! Backend-owned BIM query and read-model service.
//!
//! This module borrows the current semantic snapshot. It does not author the
//! stage, mutate USD hierarchy, or run work on a Bevy render/update system.

mod classification;
mod properties;
mod search;

#[cfg(test)]
mod test_fixtures;

#[cfg(test)]
mod classification_tests;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use usd_model::{EntitySnapshot, SemanticSnapshot, SnapshotId};
use viewport_protocol::{
    BimPropertiesReadModel, BimSearchQuery, BimSearchResult, ClassificationRecipe,
    HierarchyChildrenPage, HierarchyNodeId, HierarchyReadModel, ProtocolValidationError,
    SceneAnchor, SelectionReadModel,
};

use self::classification::ClassificationIndex;
use crate::viewport::api::CurrentHierarchyProjection;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BimReadPolicy {
    pub(crate) allow_value_edit: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum BimQueryError {
    Invalid(ProtocolValidationError),
    TargetNotFound(String),
    ClassificationNodeNotFound(String),
    EntityNotFound(String),
    InvalidRegex(String),
}

impl fmt::Display for BimQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::TargetNotFound(path) => write!(formatter, "BIM target not found: {path}"),
            Self::ClassificationNodeNotFound(id) => {
                write!(formatter, "BIM classification node not found: {id}")
            }
            Self::EntityNotFound(key) => write!(formatter, "BIM entity not found: {key}"),
            Self::InvalidRegex(error) => write!(formatter, "invalid BIM regex: {error}"),
        }
    }
}

impl Error for BimQueryError {}

impl From<ProtocolValidationError> for BimQueryError {
    fn from(error: ProtocolValidationError) -> Self {
        Self::Invalid(error)
    }
}

struct ClassificationCache {
    snapshot_id: SnapshotId,
    recipe: ClassificationRecipe,
    projection: CurrentHierarchyProjection,
    build_count: u64,
}

pub(crate) struct BimReadService<'snapshot> {
    pub(super) snapshot: &'snapshot SemanticSnapshot,
    by_path: HashMap<&'snapshot str, &'snapshot EntitySnapshot>,
    classification_cache: Option<ClassificationCache>,
}

impl<'snapshot> BimReadService<'snapshot> {
    pub(crate) fn new(snapshot: &'snapshot SemanticSnapshot) -> Self {
        let mut by_path = HashMap::with_capacity(snapshot.entities.len());
        for entity in snapshot.entities.values() {
            by_path.insert(entity.prim_path.as_str(), entity);
        }
        Self {
            snapshot,
            by_path,
            classification_cache: None,
        }
    }

    pub(crate) fn read_properties(
        &self,
        selection: &SelectionReadModel,
        selection_revision: u64,
        policy: BimReadPolicy,
    ) -> Result<BimPropertiesReadModel, BimQueryError> {
        properties::read_properties(self, selection, selection_revision, policy)
    }

    pub(crate) fn classification_page(
        &mut self,
        recipe: &ClassificationRecipe,
        parent_id: Option<&HierarchyNodeId>,
        page: u32,
        page_size: u32,
    ) -> Result<HierarchyChildrenPage, BimQueryError> {
        self.ensure_classification_cache(recipe)?;
        let cache = self
            .classification_cache
            .as_ref()
            .expect("classification cache is initialized");
        cache
            .projection
            .children_page(parent_id, page, page_size)
            .map_err(|_| {
                BimQueryError::ClassificationNodeNotFound(
                    parent_id.map_or_else(|| "<root>".to_owned(), |id| id.as_str().to_owned()),
                )
            })
    }

    pub(crate) fn classification_snapshot(
        &mut self,
        recipe: &ClassificationRecipe,
    ) -> Result<Arc<HierarchyReadModel>, BimQueryError> {
        self.ensure_classification_cache(recipe)?;
        Ok(self
            .classification_cache
            .as_ref()
            .expect("classification cache is initialized")
            .projection
            .snapshot())
    }

    fn ensure_classification_cache(
        &mut self,
        recipe: &ClassificationRecipe,
    ) -> Result<(), BimQueryError> {
        recipe.validate()?;
        let needs_rebuild = self.classification_cache.as_ref().is_none_or(|cache| {
            cache.snapshot_id != self.snapshot.snapshot_id || cache.recipe != *recipe
        });
        if needs_rebuild {
            let build_count = self
                .classification_cache
                .as_ref()
                .map_or(1, |cache| cache.build_count.saturating_add(1));
            self.classification_cache = Some(ClassificationCache {
                snapshot_id: self.snapshot.snapshot_id.clone(),
                recipe: recipe.clone(),
                projection: CurrentHierarchyProjection::from_read_model(
                    ClassificationIndex::build(self.snapshot, recipe)
                        .read_model(self.snapshot, build_count),
                ),
                build_count,
            });
        }
        Ok(())
    }

    pub(crate) fn search(&self, query: &BimSearchQuery) -> Result<BimSearchResult, BimQueryError> {
        search::execute(self, query)
    }

    #[cfg(test)]
    pub(crate) fn classification_build_count(&self) -> u64 {
        self.classification_cache
            .as_ref()
            .map_or(0, |cache| cache.build_count)
    }

    pub(super) fn entity_for_anchor(
        &self,
        anchor: &SceneAnchor,
    ) -> Result<&'snapshot EntitySnapshot, BimQueryError> {
        anchor.validate()?;
        self.by_path
            .get(anchor.prim_path.as_str())
            .copied()
            .ok_or_else(|| BimQueryError::TargetNotFound(anchor.prim_path.clone()))
    }

    pub(super) fn entities(&self) -> impl Iterator<Item = &'snapshot EntitySnapshot> {
        self.snapshot.entities.values()
    }

    pub(super) fn anchor_for_entity(entity: &EntitySnapshot) -> SceneAnchor {
        SceneAnchor::active_session(entity.prim_path.clone())
    }
}
