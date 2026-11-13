//! Backend/application ownership of classification color intent and plans.

use bevy::prelude::*;
use viewport_protocol::{
    ClassificationColorEntry, ClassificationColorIntent, ClassificationColorSource,
};

use crate::viewport::api::{ActiveHierarchyProvider, BimClassificationRecipeState};
use crate::viewport::bim::BimReadService;
use crate::viewport::semantic::SemanticSyncState;

/// Backend-generated temporary presentation plan. Its entries are never part
/// of the browser command contract.
#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct ClassificationColorPlan {
    pub(in crate::viewport) intent: Option<ClassificationColorIntent>,
    pub(in crate::viewport) generation: u64,
    pub(in crate::viewport) revision: u64,
    pub(in crate::viewport) intent_revision: u64,
    pub(in crate::viewport) entries: Vec<ClassificationColorEntry>,
}

impl ClassificationColorPlan {
    #[cfg(test)]
    pub(in crate::viewport) fn replace(
        &mut self,
        generation: u64,
        entries: Vec<ClassificationColorEntry>,
    ) {
        self.replace_entries(entries);
        self.generation = generation;
    }

    pub(in crate::viewport) fn accept_intent(
        &mut self,
        intent: ClassificationColorIntent,
    ) -> Result<bool, &'static str> {
        if let Some(current) = &self.intent {
            if intent.generation < current.generation {
                return Err("classification color intent is older than the active generation");
            }
            if *current == intent {
                return Ok(false);
            }
        }
        self.intent = Some(intent);
        self.intent_revision = self.intent_revision.saturating_add(1);
        Ok(true)
    }

    pub(in crate::viewport) fn replace_entries(&mut self, entries: Vec<ClassificationColorEntry>) {
        if self.entries == entries {
            return;
        }
        self.generation = self.intent.as_ref().map_or(0, |intent| intent.generation);
        self.entries = entries;
        self.revision = self.revision.saturating_add(1);
    }

    pub(in crate::viewport) fn entries(&self) -> &[ClassificationColorEntry] {
        &self.entries
    }

    pub(in crate::viewport) fn generation(&self) -> u64 {
        self.generation
    }

    pub(in crate::viewport) fn revision(&self) -> u64 {
        self.revision
    }

    pub(in crate::viewport) fn intent_revision(&self) -> u64 {
        self.intent_revision
    }

    pub(in crate::viewport) fn intent(&self) -> Option<ClassificationColorIntent> {
        self.intent.clone()
    }
}

/// Rebuilds the complete semantic color plan only when intent, classification
/// recipe, or legacy provider inputs change. The BIM recipe is independent of
/// the contextual hierarchy provider; browser hierarchy paging never
/// participates.
pub(in crate::viewport) fn refresh_classification_color_plan(
    provider: Option<Res<ActiveHierarchyProvider>>,
    bim_classification: Option<Res<BimClassificationRecipeState>>,
    semantic: Option<Res<SemanticSyncState>>,
    mut plan: ResMut<ClassificationColorPlan>,
    mut last_intent_revision: Local<u64>,
) {
    let intent_changed = *last_intent_revision != plan.intent_revision();
    let provider_changed = provider.as_ref().is_some_and(|value| value.is_changed());
    let classification_changed = bim_classification
        .as_ref()
        .is_some_and(|value| value.is_changed());
    if !intent_changed && !provider_changed && !classification_changed {
        return;
    }
    *last_intent_revision = plan.intent_revision();

    let Some(intent) = plan.intent() else {
        return;
    };
    let entries = if matches!(intent.source, ClassificationColorSource::None) {
        Vec::new()
    } else {
        let recipe = bim_classification
            .as_deref()
            .and_then(BimClassificationRecipeState::recipe)
            .or_else(|| {
                provider
                    .as_deref()
                    .filter(|provider| {
                        provider.source() == viewport_protocol::HierarchySource::BimClassification
                    })
                    .and_then(ActiveHierarchyProvider::classification_recipe)
            });
        let Some(recipe) = recipe else {
            plan.replace_entries(Vec::new());
            return;
        };
        let Some((snapshot, index)) = semantic
            .as_deref()
            .and_then(|state| Some((state.snapshot()?, state.shared_bim_index()?)))
        else {
            plan.replace_entries(Vec::new());
            return;
        };
        match BimReadService::with_index(snapshot, index)
            .classification_color_entries(recipe, &intent)
        {
            Ok(entries) => entries,
            Err(error) => {
                bevy::log::warn!(
                    error = %error,
                    "classification color intent could not be materialized"
                );
                Vec::new()
            }
        }
    };
    plan.replace_entries(entries);
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/scene/classification_color_plan_inline_tests.rs"]
mod tests;
