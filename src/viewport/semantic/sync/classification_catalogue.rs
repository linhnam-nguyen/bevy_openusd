use bevy::prelude::World;

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::bim::BimClassificationFieldCatalogueState;
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimFieldKey, ViewportEvent, ViewportEventEnvelope,
};

pub(super) fn publish(world: &mut World, catalogue: BimClassificationFieldCatalogue) {
    let changed = world
        .get_resource_mut::<BimClassificationFieldCatalogueState>()
        .is_some_and(|mut state| state.replace(catalogue.clone()));
    if changed && let Some(mut outbox) = world.get_resource_mut::<ViewportEventOutbox>() {
        let semantic_aliases = catalogue
            .fields
            .iter()
            .filter(|field| {
                matches!(
                    field.key,
                    BimFieldKey::Category | BimFieldKey::Family | BimFieldKey::Type
                )
            })
            .count();
        bevy::log::info!(
            "[bim-catalogue] revision={} fields={} semantic_aliases={}",
            catalogue.semantic_revision,
            catalogue.fields.len(),
            semantic_aliases
        );
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::BimClassificationFieldCatalogueChanged { catalogue },
        ));
    }
}
