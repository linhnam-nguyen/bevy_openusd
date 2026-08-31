//! Backend-owned BIM query and read-model service.
//!
//! This module borrows the current semantic snapshot. It does not author the
//! stage, mutate USD hierarchy, or run work on a Bevy render/update system.

pub(crate) mod authoring;
mod classification;
mod classification_color;
pub(super) mod diff;
mod properties;
mod search;

#[cfg(test)]
pub(crate) mod test_fixtures;

#[cfg(test)]
mod classification_tests;

#[cfg(test)]
mod classification_contract_tests;

#[cfg(test)]
mod classification_real_fixture_tests;

#[cfg(test)]
mod m8_performance_tests;

#[cfg(test)]
mod m8_failure_tests;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use usd_model::{EntitySnapshot, SemanticSnapshot, SnapshotId};
use viewport_protocol::{
    BimPropertiesReadModel, BimSearchQuery, BimSearchResult, ClassificationColorEntry,
    ClassificationColorIntent, ClassificationRecipe, HierarchyChildrenPage, HierarchyNodeId,
    HierarchyReadModel, ProtocolValidationError, SceneAnchor, SelectionReadModel,
};

use self::classification::ClassificationIndex;
use crate::viewport::api::{CurrentHierarchyProjection, HierarchyPageIndex};

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
    TooManyResults { kind: &'static str, limit: usize },
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
            Self::TooManyResults { kind, limit } => {
                write!(
                    formatter,
                    "BIM {kind} search exceeds bounded group limit {limit}"
                )
            }
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
    read_model: Arc<HierarchyReadModel>,
    page_index: HierarchyPageIndex,
    color_groups: Arc<Vec<classification::ClassificationColorGroup>>,
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

    pub(crate) fn property_diff(
        &self,
        baseline: &SemanticSnapshot,
        selection: &[SceneAnchor],
    ) -> Option<viewport_protocol::BimPropertyDiffReadModel> {
        diff::property_diff(baseline, self.snapshot, selection)
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
            .page_index
            .children_page(&cache.read_model, parent_id, page, page_size)
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
        Ok(Arc::clone(
            &self
                .classification_cache
                .as_ref()
                .expect("classification cache is initialized")
                .read_model,
        ))
    }

    pub(crate) fn classification_projection(
        &mut self,
        recipe: &ClassificationRecipe,
    ) -> Result<CurrentHierarchyProjection, BimQueryError> {
        self.ensure_classification_cache(recipe)?;
        let cache = self
            .classification_cache
            .take()
            .expect("classification cache is initialized");
        Ok(CurrentHierarchyProjection::from_shared_parts(
            cache.read_model,
            cache.page_index,
        ))
    }

    pub(crate) fn classification_color_entries(
        &mut self,
        recipe: &ClassificationRecipe,
        intent: &ClassificationColorIntent,
    ) -> Result<Vec<ClassificationColorEntry>, BimQueryError> {
        recipe.validate()?;
        if !matches!(
            intent.source,
            viewport_protocol::ClassificationColorSource::None
        ) && !recipe
            .levels
            .iter()
            .any(|level| Some(level.id.as_str()) == intent.active_level.as_deref())
        {
            return Err(BimQueryError::Invalid(
                ProtocolValidationError::InvalidInput {
                    field: "classification_color.active_level",
                },
            ));
        }
        self.ensure_classification_cache(recipe)?;
        let cache = self
            .classification_cache
            .as_ref()
            .expect("classification cache is initialized");
        classification_color::entries(&cache.color_groups, intent)
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
            let index = ClassificationIndex::build(self.snapshot, recipe);
            let color_groups = Arc::new(index.color_groups().to_vec());
            let read_model = Arc::new(index.read_model(self.snapshot, build_count));
            let page_index = HierarchyPageIndex::from_read_model(&read_model);
            self.classification_cache = Some(ClassificationCache {
                snapshot_id: self.snapshot.snapshot_id.clone(),
                recipe: recipe.clone(),
                read_model,
                page_index,
                color_groups,
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
        self.snapshot
            .entities
            .values()
            .filter(|entity| entity.semantic.is_bim_entity())
    }

    pub(super) fn anchor_for_entity(entity: &EntitySnapshot) -> SceneAnchor {
        SceneAnchor::active_session(entity.prim_path.clone())
    }
}
