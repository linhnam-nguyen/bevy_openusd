//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).

mod animation;
mod author;
mod change;
mod index;
mod native_instance_dependency;
mod path;
mod performance;
mod progressive;
mod progressive_cleanup;
mod progressive_resident;
mod progressive_state;
mod projection;
mod projection_plan;
mod reconcile;
mod stage;
mod system;

pub use animation::AnimatedPrims;
pub use author::{TransformHistory, author_transform, current_transform};
pub use change::{LiveRevision, PendingStageChanges, StageChange, StageChangeBatch};
pub use index::PrimEntities;
pub use native_instance_dependency::NativeInstanceDependencyIndex;
pub use path::{
    is_descendant_or_self, minimize_resync_roots, normalize_prim_path, prim_of, property_of,
    validate_prim_path,
};
pub use performance::PerformanceCounters;
pub use progressive_state::{ProgressiveProjectionState, ProjectionBudget, ProjectionReadiness};
pub use projection::{ProjectionStats, collect_stage_subtree_paths, project_stage};
pub use projection_plan::{ProjectionPlan, ProjectionPlanBuilder, ProjectionPlanEntry};
pub(crate) use reconcile::ReconcileStats;
pub use reconcile::{apply_change_batch, apply_changes};
pub use stage::{AuthoredSuppressionGuard, LiveStage};

use bevy::app::{App, Plugin, Update};
use bevy::prelude::*;

use crate::prim_ref::SemanticEntityIndex;
use crate::route::{SchemaRegistry, StageTime};
use animation::{SampledTime, resample_animation_system};
use progressive::project_on_load_system;
use system::{
    AppliedPurposes, apply_display_purposes_system, drain_stage_changes_system,
    reproject_from_batch_system,
};

/// Registers the `PrimEntities` bimap and the per-frame reprojection system.
/// Insert a `LiveStage` non-send resource to begin a live session.
pub struct LiveStagePlugin;

/// Ordering boundary for systems that consume the live-stage projection.
///
/// Viewport systems that must capture transient entity identity before a
/// destructive reconciliation should order themselves before
/// [`LiveStageSet::Reconcile`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveStageSet {
    Project,
    Drain,
    Reconcile,
    Animation,
    Presentation,
}

impl Plugin for LiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrimEntities>()
            .init_resource::<ProjectionBudget>()
            .init_resource::<ProgressiveProjectionState>()
            .init_resource::<SemanticEntityIndex>()
            .init_resource::<PendingStageChanges>()
            .init_resource::<ReconcileStats>()
            .init_resource::<StageTime>()
            .init_resource::<AnimatedPrims>()
            .init_resource::<PerformanceCounters>()
            .init_resource::<NativeInstanceDependencyIndex>()
            .init_resource::<SampledTime>()
            .init_resource::<crate::route::DisplayPurposes>()
            .init_resource::<AppliedPurposes>()
            .configure_sets(
                Update,
                (
                    LiveStageSet::Project,
                    LiveStageSet::Drain.after(LiveStageSet::Project),
                    LiveStageSet::Reconcile.after(LiveStageSet::Drain),
                    LiveStageSet::Animation.after(LiveStageSet::Reconcile),
                    LiveStageSet::Presentation.after(LiveStageSet::Animation),
                ),
            )
            .add_systems(Update, project_on_load_system.in_set(LiveStageSet::Project))
            .add_systems(
                Update,
                drain_stage_changes_system.in_set(LiveStageSet::Drain),
            )
            .add_systems(
                Update,
                reproject_from_batch_system.in_set(LiveStageSet::Reconcile),
            )
            .add_systems(
                Update,
                resample_animation_system.in_set(LiveStageSet::Animation),
            )
            .add_systems(
                Update,
                apply_display_purposes_system.in_set(LiveStageSet::Presentation),
            );
        // Ensure the routing registry exists even if `UsdPlugin` wasn't added.
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
    }
}

#[cfg(test)]
mod tests;
