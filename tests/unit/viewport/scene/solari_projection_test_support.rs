//! Test-only helpers attached to the owning production module.
use super::*;

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SolariProjectionStats {
    pub(crate) full_scans: u32,
    pub(crate) incremental_entities: u32,
}

impl SolariProjectionStats {
    pub(super) fn record_incremental(&mut self) {
        self.incremental_entities += 1;
    }
    pub(super) fn record_full_scan(&mut self) {
        self.full_scans += 1;
    }
}

impl SolariProjectionState {
    pub(crate) fn initialized_for_test() -> Self {
        Self {
            initialized: true,
            device_supported: true,
            ..Default::default()
        }
    }
}

#[cfg(feature = "solari")]
struct CountingSolariWork<'w>(Option<ResMut<'w, SolariProjectionStats>>);

#[cfg(feature = "solari")]
impl ProjectionWork for CountingSolariWork<'_> {
    fn execute<I, F>(&mut self, batch: ProjectionBatch, items: I, mut apply: F)
    where
        I: IntoIterator,
        F: FnMut(I::Item),
    {
        for item in items {
            apply(item);
            if let Some(stats) = self.0.as_deref_mut() {
                match batch {
                    ProjectionBatch::SolariFullScan => stats.record_full_scan(),
                    ProjectionBatch::SolariIncremental => stats.record_incremental(),
                    _ => unreachable!("unexpected Solari work batch"),
                }
            }
        }
    }
}

#[cfg(feature = "solari")]
#[allow(clippy::too_many_arguments)]
pub(in crate::viewport::scene::solari) fn sync_solari_usd_projection_counted(
    capability: ResMut<SolariCapability>,
    toggles: Res<DisplayToggles>,
    meshes: Option<Res<Assets<Mesh>>>,
    state: ResMut<SolariProjectionState>,
    diagnostics: ResMut<SolariProjectionDiagnostics>,
    commands: Commands,
    projection_query: Query<
        (
            Entity,
            &Mesh3d,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&bevy::solari::prelude::RaytracingMesh3d>,
        ),
        With<UsdPrimRef>,
    >,
    marked_entities: Query<
        Entity,
        (
            With<UsdPrimRef>,
            With<bevy::solari::prelude::RaytracingMesh3d>,
        ),
    >,
    removed_prims: RemovedComponents<UsdPrimRef>,
    removed_meshes: RemovedComponents<Mesh3d>,
    removed_materials: RemovedComponents<MeshMaterial3d<StandardMaterial>>,
    mesh_asset_events: Option<MessageReader<AssetEvent<Mesh>>>,
    dirty_set: Option<ResMut<RenderProjectionDirtySet>>,
    mesh_consumers: Option<Res<MeshProjectionConsumers>>,
    stats: Option<ResMut<SolariProjectionStats>>,
) {
    sync_solari_usd_projection_core(
        capability,
        toggles,
        meshes,
        state,
        diagnostics,
        commands,
        projection_query,
        marked_entities,
        removed_prims,
        removed_meshes,
        removed_materials,
        mesh_asset_events,
        dirty_set,
        mesh_consumers,
        &mut CountingSolariWork(stats),
    );
}
