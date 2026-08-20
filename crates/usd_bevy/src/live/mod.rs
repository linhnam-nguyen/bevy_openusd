//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).

mod animation;
mod author;
mod change;
mod index;
mod path;
mod projection;
mod reconcile;
mod stage;
mod system;

pub use animation::AnimatedPrims;
pub use author::{TransformHistory, author_transform, current_transform};
pub use change::{LiveRevision, PendingStageChanges, StageChange, StageChangeBatch};
pub use index::PrimEntities;
pub use path::{
    is_descendant_or_self, minimize_resync_roots, normalize_prim_path, prim_of, property_of,
    validate_prim_path,
};
pub use projection::{ProjectionStats, collect_stage_subtree_paths, project_stage};
pub(crate) use reconcile::ReconcileStats;
pub use reconcile::{apply_change_batch, apply_changes};
pub use stage::LiveStage;

use bevy::app::{App, Plugin, Update};
use bevy::prelude::*;

use crate::prim_ref::SemanticEntityIndex;
use crate::route::{SchemaRegistry, StageTime};
use animation::{SampledTime, resample_animation_system};
use projection::project_on_load_system;
use system::{
    AppliedPurposes, apply_display_purposes_system, drain_stage_changes_system,
    reproject_from_batch_system,
};

/// Registers the `PrimEntities` bimap and the per-frame reprojection system.
/// Insert a `LiveStage` non-send resource to begin a live session.
pub struct LiveStagePlugin;

impl Plugin for LiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrimEntities>()
            .init_resource::<SemanticEntityIndex>()
            .init_resource::<PendingStageChanges>()
            .init_resource::<ReconcileStats>()
            .init_resource::<StageTime>()
            .init_resource::<AnimatedPrims>()
            .init_resource::<SampledTime>()
            .init_resource::<crate::route::DisplayPurposes>()
            .init_resource::<AppliedPurposes>()
            .add_systems(
                Update,
                (
                    project_on_load_system,
                    drain_stage_changes_system,
                    reproject_from_batch_system,
                    resample_animation_system,
                    apply_display_purposes_system,
                )
                    .chain(),
            );
        // Ensure the routing registry exists even if `UsdPlugin` wasn't added.
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
    }
}

#[cfg(test)]
mod tests;
