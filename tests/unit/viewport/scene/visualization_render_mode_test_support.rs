use super::*;

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport::scene) struct RenderModeProjectionStats {
    pub(super) full_transition_scans: u32,
    pub(super) incremental_scans: u32,
    pub(super) restore_scans: u32,
}

impl RenderModeProjectionStats {
    pub(super) fn record_transition(&mut self) {
        self.full_transition_scans += 1;
    }
    pub(super) fn record_restore(&mut self) {
        self.restore_scans += 1;
    }
    pub(super) fn record_incremental(&mut self) {
        self.incremental_scans += 1;
    }
}

struct CountingRenderModeWork<'w>(Option<ResMut<'w, RenderModeProjectionStats>>);

impl ProjectionWork for CountingRenderModeWork<'_> {
    fn execute<I, F>(&mut self, batch: ProjectionBatch, items: I, mut apply: F)
    where
        I: IntoIterator,
        F: FnMut(I::Item),
    {
        for item in items {
            if let Some(stats) = self.0.as_mut() {
                match batch {
                    ProjectionBatch::MaterialTransition => stats.record_transition(),
                    ProjectionBatch::MaterialRestore => stats.record_restore(),
                    ProjectionBatch::MaterialIncremental => stats.record_incremental(),
                    _ => unreachable!("unexpected render-mode work batch"),
                }
            }
            apply(item);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_render_mode_counted(
    toggles: Res<DisplayToggles>,
    uniform: Option<Res<UniformRenderMaterial>>,
    projection: ResMut<RenderModeProjectionState>,
    commands: Commands,
    prims: ParamSet<(
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<UsdPrimRef>,
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                Option<&mut OriginalRenderMaterial>,
                Option<&mut SelectionBaseMaterial>,
            ),
            (
                With<UsdPrimRef>,
                Or<(Added<UsdPrimRef>, Changed<MeshMaterial3d<StandardMaterial>>)>,
            ),
        >,
        Query<
            (
                Entity,
                &mut MeshMaterial3d<StandardMaterial>,
                &OriginalRenderMaterial,
                Option<&mut SelectionBaseMaterial>,
            ),
            With<OriginalRenderMaterial>,
        >,
    )>,
    stats: Option<ResMut<RenderModeProjectionStats>>,
) {
    apply_render_mode_core(
        toggles,
        uniform,
        projection,
        commands,
        prims,
        &mut CountingRenderModeWork(stats),
    );
}
