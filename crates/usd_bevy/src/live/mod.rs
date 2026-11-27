//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).

mod animation;
mod author;
mod change;
mod deformed_sort;
mod index;
mod native_animation;
mod native_instance_dependency;
mod path;
mod performance;
mod progressive;
mod progressive_cleanup;
mod progressive_resident;
mod progressive_state;
mod project_readiness;
mod projection;
mod projection_plan;
mod reconcile;
pub mod source_mesh;
mod stage;
mod system;
pub(crate) mod visual_stage;

pub use animation::AnimatedPrims;
pub use author::{TransformHistory, author_transform, current_transform};
pub use change::{
    LiveRevision, PendingStageChanges, PersistedChangeSnapshot, StageChange, StageChangeBatch,
};
pub use deformed_sort::UsdDeformedSortPlugin;
pub use index::PrimEntities;
pub use native_instance_dependency::NativeInstanceDependencyIndex;
pub(crate) use native_instance_dependency::native_proxy_sources;
pub use path::{
    PathId, PathStore, is_descendant_or_self, minimize_resync_roots, normalize_prim_path, prim_of,
    property_of, validate_prim_path,
};
pub use performance::PerformanceCounters;
pub use progressive_state::{ProgressiveProjectionState, ProjectionBudget, ProjectionReadiness};
pub use project_readiness::{ProjectOpenReadiness, ProjectOpenReadinessState};
pub use projection::{
    ProjectionStats, TargetedRenderPayload, collect_stage_subtree_paths,
    extract_render_payloads_for_paths, project_paths, project_stage,
};
pub use projection_plan::{ProjectionPlan, ProjectionPlanBuilder, ProjectionPlanEntry};
pub use reconcile::{apply_change_batch, apply_changes};
pub use stage::{AuthoredSuppressionGuard, LiveStage, VisualContextId};

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

/// Reconcile existing PointInstancers when the current presentation policy
/// switches between packed raster and the regular entity representation.
pub fn sync_point_instancing_presentation(
    world: &mut World,
    compatible: bool,
    selected: std::collections::BTreeMap<String, Vec<(i64, u64)>>,
) {
    use crate::route::gpu_instancing::{
        PROMOTION_BUDGET, PointPresentations, SelectionOnlyPointProjection, UsdGpuInstancing,
    };
    use crate::route::instancer::PointInstancerRoute;
    use crate::route::instancer_dependency::PointInstancerDependencyIndex;
    use crate::route::{PrimRoute, RouteCtx};

    let Some(mut policy) = world.get_resource_mut::<UsdGpuInstancing>() else {
        return;
    };
    let compatibility_changed = policy.presentation_compatible != compatible;
    policy.presentation_compatible = compatible;
    let over_budget = selected.values().map(Vec::len).sum::<usize>() > PROMOTION_BUDGET;
    let previous = world.resource::<PointPresentations>();
    let next = PointPresentations {
        selected,
        over_budget,
    };
    if !compatibility_changed && *previous == next {
        return;
    }
    let selection_only = !compatibility_changed && !previous.over_budget && !next.over_budget;
    let changed_paths: std::collections::HashSet<_> = previous
        .selected
        .keys()
        .chain(next.selected.keys())
        .filter(|path| {
            previous.over_budget != next.over_budget
                || previous.selected.get(*path) != next.selected.get(*path)
        })
        .cloned()
        .collect();
    world.insert_resource(next);
    let Some(stage) = world
        .get_non_send::<LiveStage>()
        .map(LiveStage::projection_stage)
    else {
        return;
    };
    let paths = world.resource::<PathStore>();
    let entities = world.resource::<PrimEntities>();
    let targets: Vec<_> = world
        .resource::<PointInstancerDependencyIndex>()
        .instancers()
        .filter_map(|id| {
            let resolved = paths.path(id)?;
            if !compatibility_changed && !changed_paths.contains(resolved) {
                return None;
            }
            let path = openusd::sdf::path(resolved).ok()?;
            Some((path, entities.entity_id(id)?))
        })
        .collect();
    let time = world.get_resource::<StageTime>().map(|time| time.current);
    bevy::log::info!(
        target: "usd_bevy::route::gpu_instancing",
        compatible,
        instancers = targets.len(),
        "reconciling point-instancer presentation for viewport settings"
    );
    if selection_only {
        world.insert_resource(SelectionOnlyPointProjection);
    }
    for (path, entity) in targets {
        PointInstancerRoute.project(&RouteCtx::at(&stage, &path, time), world, entity);
    }
    if selection_only {
        world.remove_resource::<SelectionOnlyPointProjection>();
    }
}

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
        Self::build_with::<reconcile::WorldReconcileExecutor>(app);
    }
}

impl LiveStagePlugin {
    fn build_with<E: reconcile::ReconcileExecutor + Default + 'static>(app: &mut App) {
        app.init_resource::<PathStore>()
            .init_resource::<PrimEntities>()
            .init_resource::<ProjectionBudget>()
            .init_resource::<ProgressiveProjectionState>()
            .init_resource::<ProjectOpenReadinessState>()
            .init_resource::<SemanticEntityIndex>()
            .init_resource::<PendingStageChanges>()
            .init_resource::<StageTime>()
            .init_resource::<AnimatedPrims>()
            .init_resource::<PerformanceCounters>()
            .init_resource::<NativeInstanceDependencyIndex>()
            .init_resource::<SampledTime>()
            .init_non_send::<native_animation::AnimationRuntime>()
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
                project_readiness::observe_project_open_readiness
                    .in_set(LiveStageSet::Presentation),
            )
            .add_systems(
                Update,
                drain_stage_changes_system.in_set(LiveStageSet::Drain),
            )
            .add_systems(
                Update,
                reproject_from_batch_system::<E>.in_set(LiveStageSet::Reconcile),
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
#[path = "../../tests/unit/live/mod.rs"]
mod tests;
