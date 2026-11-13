//! External observation of actual reconciliation work. Only the counted unit
//! harness creates or publishes this resource; production executes the same
//! work without measurements.

use bevy::prelude::*;
use openusd::sdf::Path;
use openusd::usd::Stage;

use super::{
    FullWork, ReconcileExecutor, SparseWork, SubtreeWork, WorldReconcileExecutor,
    apply_change_batch_with_executor, execute_sparse_work, full, subtree,
};
use crate::live::{LiveStage, LiveStagePlugin, PrimEntities, StageChangeBatch};
use crate::route::SchemaRegistry;

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::live) struct ReconcileStats {
    pub(in crate::live) roots: usize,
    pub(in crate::live) visited_stage_prims: usize,
    pub(in crate::live) patched_entities: usize,
    pub(in crate::live) spawned_entities: usize,
    pub(in crate::live) despawned_entities: usize,
}

#[derive(Default)]
struct CountingExecutor {
    patched: usize,
    spawned: usize,
}

impl CountingExecutor {
    fn reset_pass(&mut self) {
        self.patched = 0;
        self.spawned = 0;
    }

    fn snapshot(&self, roots: usize, visited: usize, despawned: usize) -> ReconcileStats {
        ReconcileStats {
            roots,
            visited_stage_prims: visited,
            patched_entities: self.patched,
            spawned_entities: self.spawned,
            despawned_entities: despawned,
        }
    }
}

impl ReconcileExecutor for CountingExecutor {
    fn execute_full(
        &mut self,
        world: &mut World,
        live: &LiveStage,
        map: &mut PrimEntities,
        work: FullWork<'_>,
    ) {
        let visited = work.current.len();
        let despawned = work.stale.len();
        self.reset_pass();
        full::execute_full_work(world, live, map, work, self);
        // Match the previous full-pass publication point: animation has been
        // rebuilt; the caller still owns native dependency rebuild.
        world.insert_resource(self.snapshot(1, visited, despawned));
    }

    fn execute_subtree(
        &mut self,
        world: &mut World,
        live: &LiveStage,
        map: &mut PrimEntities,
        work: SubtreeWork<'_>,
    ) -> bool {
        let roots = work.roots.len();
        let visited = work.current_paths.len();
        let despawned = work.removed.len();
        self.reset_pass();
        let completed = subtree::execute_subtree_work(world, live, map, work, self);
        // A partial subtree pass requests the same full fallback; never
        // publish its incomplete snapshot. A failed full pass leaves the
        // preceding snapshot intact, as in the original implementation.
        if completed {
            world.insert_resource(self.snapshot(roots, visited, despawned));
        }
        completed
    }

    fn execute_sparse(
        &mut self,
        world: &mut World,
        live: &LiveStage,
        map: &PrimEntities,
        work: SparseWork<'_, '_>,
    ) {
        self.reset_pass();
        execute_sparse_work(world, live, map, work, self);
        // Sparse semantics are intentionally patches, including actual
        // prototype/native dependents, not an independent stage scan.
        world.insert_resource(self.snapshot(0, self.patched, 0));
    }

    fn patch_prim(
        &mut self,
        registry: &SchemaRegistry,
        stage: &Stage,
        path: &Path,
        world: &mut World,
        entity: Entity,
        properties: &[&str],
    ) {
        WorldReconcileExecutor.patch_prim(registry, stage, path, world, entity, properties);
        self.patched += 1;
    }

    fn project_prim(
        &mut self,
        registry: &SchemaRegistry,
        stage: &Stage,
        path: &Path,
        world: &mut World,
        entity: Entity,
    ) {
        WorldReconcileExecutor.project_prim(registry, stage, path, world, entity);
        self.spawned += 1;
    }
}

/// Uses exactly the same plugin configuration and exclusive system, selecting
/// the external counted executor. The normal plugin never initializes stats.
pub(in crate::live) struct CountingLiveStagePlugin;

impl Plugin for CountingLiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReconcileStats>();
        LiveStagePlugin::build_with::<CountingExecutor>(app);
    }
}

/// Direct already-drained-batch path for existing unit fixtures.
/// The actual dispatch is shared with the normal public API.
pub(in crate::live) fn apply_counted_change_batch(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    batch: &StageChangeBatch,
) {
    world.init_resource::<ReconcileStats>();
    apply_change_batch_with_executor(world, live, map, batch, &mut CountingExecutor::default());
}
