//! Visible coarse-selection proxy owned by the renderer.

use bevy::prelude::*;

use crate::viewport::api::SceneAnchorIndex;

use super::{
    ProjectedWorldBounds, SelectedRenderableProjection, SelectedTargets,
    SelectionPresentationPolicy,
};

const COARSE_PROXY_COLOR: Color = Color::srgba(1.0, 0.66, 0.1, 0.95);
const ROOT_PROXY_HALF_EXTENT: f32 = 0.25;

/// The renderer-owned, durable state for exactly one coarse selection proxy.
/// It is deliberately independent from per-mesh outline/color components.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub(in crate::viewport) struct CoarseSelectionProxyState {
    pub(in crate::viewport) visible: bool,
    pub(in crate::viewport) bounds: Option<ProjectedWorldBounds>,
    pub(in crate::viewport) generation: u64,
}

pub(in crate::viewport) fn sync_coarse_selection_proxy(
    selection: Res<SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    policy: Res<SelectionPresentationPolicy>,
    projection: Res<SelectedRenderableProjection>,
    roots: Query<&GlobalTransform>,
    mut state: ResMut<CoarseSelectionProxyState>,
) {
    let coarse = policy.uses_coarse(projection.renderables().len());
    let bounds = projection
        .aggregate_bounds()
        .or_else(|| root_proxy_bounds(&selection, &scene_index, &roots));
    let visible = coarse && !selection.0.targets.is_empty() && bounds.is_some();
    if state.visible != visible || state.bounds != bounds {
        state.visible = visible;
        state.bounds = bounds;
        state.generation = state.generation.saturating_add(1);
    }
}

/// Draws a real aggregate proxy even when the selected root has no mesh. The
/// proxy is one renderer-owned wire box, not a synthetic mesh per target.
pub(in crate::viewport) fn draw_coarse_selection_proxy(
    state: Res<CoarseSelectionProxyState>,
    mut gizmos: Gizmos,
) {
    let Some(bounds) = state.bounds else {
        return;
    };
    if !state.visible {
        return;
    }
    for (start, end) in proxy_edges(bounds) {
        gizmos.line(start, end, COARSE_PROXY_COLOR);
    }
}

fn root_proxy_bounds(
    selection: &SelectedTargets,
    scene_index: &SceneAnchorIndex,
    roots: &Query<&GlobalTransform>,
) -> Option<ProjectedWorldBounds> {
    selection
        .0
        .targets
        .iter()
        .filter_map(|target| scene_index.resolve(target))
        .filter_map(|root| roots.get(root).ok())
        .map(|transform| {
            let center = transform.translation();
            ProjectedWorldBounds {
                min: center - Vec3::splat(ROOT_PROXY_HALF_EXTENT),
                max: center + Vec3::splat(ROOT_PROXY_HALF_EXTENT),
            }
        })
        .reduce(|mut aggregate, next| {
            aggregate.min = aggregate.min.min(next.min);
            aggregate.max = aggregate.max.max(next.max);
            aggregate
        })
}

fn proxy_edges(bounds: ProjectedWorldBounds) -> [(Vec3, Vec3); 12] {
    let corners = [
        Vec3::new(bounds.min.x, bounds.min.y, bounds.min.z),
        Vec3::new(bounds.max.x, bounds.min.y, bounds.min.z),
        Vec3::new(bounds.max.x, bounds.max.y, bounds.min.z),
        Vec3::new(bounds.min.x, bounds.max.y, bounds.min.z),
        Vec3::new(bounds.min.x, bounds.min.y, bounds.max.z),
        Vec3::new(bounds.max.x, bounds.min.y, bounds.max.z),
        Vec3::new(bounds.max.x, bounds.max.y, bounds.max.z),
        Vec3::new(bounds.min.x, bounds.max.y, bounds.max.z),
    ];
    [
        (corners[0], corners[1]),
        (corners[1], corners[2]),
        (corners[2], corners[3]),
        (corners[3], corners[0]),
        (corners[4], corners[5]),
        (corners[5], corners[6]),
        (corners[6], corners[7]),
        (corners[7], corners[4]),
        (corners[0], corners[4]),
        (corners[1], corners[5]),
        (corners[2], corners[6]),
        (corners[3], corners[7]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_proxy_has_one_wire_box() {
        let edges = proxy_edges(ProjectedWorldBounds {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        });
        assert_eq!(edges.len(), 12);
    }
}
