use bevy::prelude::{Resource, World};
use viewport_protocol::{ClassificationRecipe, SelectionReadModel};

/// Presentation intent captured at the activation boundary and consumed only
/// after the matching semantic/BIM generation is observable.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub(super) struct PendingActivationPresentation {
    pub(super) generation: u64,
    pub(super) classification_recipe: Option<ClassificationRecipe>,
    pub(super) selection: Option<SelectionReadModel>,
}

/// Clears all derived state that is scoped to the stage being replaced.
/// Source projection starts again from the newly activated LiveStage, while
/// the selected hierarchy provider remains a user presentation preference.
pub(super) fn reset_derived_state(world: &mut World, activation_generation: u64) {
    let classification_recipe = world
        .get_resource::<crate::viewport::api::BimClassificationRecipeState>()
        .and_then(|state| state.recipe().cloned())
        .or_else(|| {
            world
                .get_resource::<crate::viewport::api::ActiveHierarchyProvider>()
                .and_then(|provider| provider.classification_recipe().cloned())
        });
    let selection = world
        .get_resource::<crate::viewport::scene::SelectedTargets>()
        .map(|selected| selected.0.clone())
        .filter(|selection| !selection.targets.is_empty());
    world.insert_resource(PendingActivationPresentation {
        generation: activation_generation,
        classification_recipe,
        selection,
    });

    if let Some(mut semantic) =
        world.get_resource_mut::<crate::viewport::semantic::SemanticSyncState>()
    {
        semantic.reset_for_activation(activation_generation);
    }
    if let Some(mut diff) = world.get_resource_mut::<crate::viewport::semantic::SemanticDiffState>()
    {
        diff.reset_for_activation();
    }
    if let Some(mut catalogue) =
        world.get_resource_mut::<crate::viewport::bim::BimClassificationFieldCatalogueState>()
    {
        catalogue.clear();
    }
    if let Some(mut classification) =
        world.get_resource_mut::<crate::viewport::api::BimClassificationRecipeState>()
    {
        classification.set(None);
    }

    let hierarchy_source = world
        .get_resource::<crate::viewport::api::ActiveHierarchyProvider>()
        .map_or(viewport_protocol::HierarchySource::Prim, |provider| {
            provider.source()
        });
    if let Some(mut projection) =
        world.get_resource_mut::<crate::viewport::api::CurrentHierarchyProjection>()
    {
        *projection = crate::viewport::api::CurrentHierarchyProjection::empty(hierarchy_source, 0);
    }
    if let Some(mut selected) = world.get_resource_mut::<crate::viewport::scene::SelectedTargets>()
    {
        let _ = selected.clear();
    }
    if let Some(mut selected_prim) =
        world.get_resource_mut::<crate::viewport::scene::SelectedPrim>()
    {
        selected_prim.0 = None;
    }
}

/// Reconciles retained presentation intent after the current generation has
/// produced both semantic/BIM state and a usable scene-anchor index. No timer
/// or blind retry is involved: the pending record remains until its source
/// generation is actually ready.
pub(in crate::viewport) fn rehydrate_activation_presentation(world: &mut World) {
    let Some(pending) = world
        .get_resource::<PendingActivationPresentation>()
        .cloned()
    else {
        return;
    };
    let Some(stage_info) = world.get_resource::<crate::viewport::session::StageInfo>() else {
        return;
    };
    if pending.generation != stage_info.activation_generation {
        return;
    }
    let semantic_ready = world
        .get_resource::<crate::viewport::semantic::SemanticSyncState>()
        .is_some_and(|semantic| {
            semantic.snapshot().is_some() && semantic.shared_bim_index().is_some()
        });
    if !semantic_ready {
        return;
    }

    let resolved_selection = pending.selection.as_ref().map(|selection| {
        let index = world.resource::<crate::viewport::api::SceneAnchorIndex>();
        let targets = selection
            .targets
            .iter()
            .filter(|target| index.resolve(target).is_some())
            .cloned()
            .collect::<Vec<_>>();
        let primary = selection
            .primary
            .as_ref()
            .filter(|primary| targets.contains(primary))
            .cloned()
            .or_else(|| targets.first().cloned());
        SelectionReadModel { targets, primary }
    });
    if resolved_selection
        .as_ref()
        .is_some_and(|selection| !selection.targets.is_empty())
        && world
            .get_resource::<crate::viewport::api::SceneAnchorIndex>()
            .is_some_and(|index| {
                resolved_selection.as_ref().is_some_and(|selection| {
                    selection
                        .targets
                        .iter()
                        .any(|target| index.resolve(target).is_some())
                })
            })
    {
        // The index has projected at least one retained target, so selection
        // can be restored without exposing a stale Bevy entity.
        let selection = resolved_selection
            .as_ref()
            .expect("resolved selection checked above");
        {
            let Some(mut selected) =
                world.get_resource_mut::<crate::viewport::scene::SelectedTargets>()
            else {
                return;
            };
            if selected.replace(selection.clone()).is_err() {
                return;
            }
        }
        let primary = selection.primary.as_ref().and_then(|target| {
            world
                .resource::<crate::viewport::api::SceneAnchorIndex>()
                .resolve(target)
        });
        if let Some(mut selected_prim) =
            world.get_resource_mut::<crate::viewport::scene::SelectedPrim>()
        {
            selected_prim.0 = primary;
        }
    } else if pending.selection.is_some()
        && world
            .get_resource::<usd_bevy::ProgressiveProjectionState>()
            .is_some_and(|state| state.readiness() == usd_bevy::ProjectionReadiness::Ready)
        && world
            .get_resource::<crate::viewport::api::SceneAnchorIndex>()
            .is_some_and(|index| index.revision() > 0)
    {
        // Only a complete projection proves that a retained target disappeared.
        // A non-empty progressive index is still an incomplete prefix.
        world.remove_resource::<PendingActivationPresentation>();
        return;
    } else if pending.selection.is_some() {
        return;
    }

    if let Some(recipe) = pending.classification_recipe {
        if let Some(mut classification) =
            world.get_resource_mut::<crate::viewport::api::BimClassificationRecipeState>()
        {
            classification.set(Some(recipe.clone()));
        }
        if let Some(mut provider) =
            world.get_resource_mut::<crate::viewport::api::ActiveHierarchyProvider>()
        {
            let source = provider.source();
            if source == viewport_protocol::HierarchySource::BimClassification
                && provider.classification_recipe().is_none()
            {
                provider.set(source, Some(recipe));
            }
        }
    }
    world.remove_resource::<PendingActivationPresentation>();
}
