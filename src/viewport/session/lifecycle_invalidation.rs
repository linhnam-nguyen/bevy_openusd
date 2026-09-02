use bevy::prelude::World;

/// Clears all derived state that is scoped to the stage being replaced.
/// Source projection starts again from the newly activated LiveStage, while
/// the selected hierarchy provider remains a user presentation preference.
pub(super) fn reset_derived_state(world: &mut World, activation_generation: u64) {
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
