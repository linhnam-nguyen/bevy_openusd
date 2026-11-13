use super::*;

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport::scene) struct ShadowProjectionStats {
    pub(super) full_light_visits: u32,
}

impl ShadowProjectionStats {
    pub(super) fn record_light(&mut self) {
        self.full_light_visits += 1;
    }
}

struct CountingShadowWork<'w>(Option<ResMut<'w, ShadowProjectionStats>>);

impl ProjectionWork for CountingShadowWork<'_> {
    fn execute<I, F>(&mut self, batch: ProjectionBatch, items: I, mut apply: F)
    where
        I: IntoIterator,
        F: FnMut(I::Item),
    {
        debug_assert!(matches!(batch, ProjectionBatch::ShadowTransition));
        for item in items {
            if let Some(stats) = self.0.as_mut() {
                stats.record_light();
            }
            apply(item);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_shadow_toggle_counted(
    toggles: Res<DisplayToggles>,
    projection: ResMut<ShadowProjectionState>,
    lights: ParamSet<(
        Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
        Query<(&mut PointLight, &OriginalShadowEnabled)>,
        Query<(&mut SpotLight, &OriginalShadowEnabled)>,
    )>,
    stats: Option<ResMut<ShadowProjectionStats>>,
) {
    apply_shadow_toggle_core(toggles, projection, lights, &mut CountingShadowWork(stats));
}
