//! Low-overhead counters for the OR3 working-set and animation baselines.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::mesh::{SkinFidelity, SkinFidelityMetrics};

/// Opt-in load-time distribution for the data-driven skinning classifier.
/// Runtime playback never updates this resource.
#[derive(Resource, Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SkinningProfile {
    pub enabled: bool,
    pub standard4_meshes: u64,
    pub extended16_meshes: u64,
    pub standard4_vertices: u64,
    pub extended16_vertices: u64,
    pub discarded_weight_sum: f64,
    pub discarded_weight_max: f64,
    /// Buckets are <=0.1%, <=1%, <=5%, <=10%, <=25%, and >25% discarded.
    pub discarded_weight_buckets: [u64; 6],
}

impl SkinningProfile {
    pub(crate) fn record(&mut self, metrics: &SkinFidelityMetrics) {
        match metrics.fidelity {
            SkinFidelity::Standard4 => {
                self.standard4_meshes = self.standard4_meshes.saturating_add(1);
                self.standard4_vertices = self
                    .standard4_vertices
                    .saturating_add(metrics.vertex_count as u64);
            }
            SkinFidelity::Extended16 => {
                self.extended16_meshes = self.extended16_meshes.saturating_add(1);
                self.extended16_vertices = self
                    .extended16_vertices
                    .saturating_add(metrics.vertex_count as u64);
            }
        }
        self.discarded_weight_sum += metrics.discarded_weight_sum;
        self.discarded_weight_max = self.discarded_weight_max.max(metrics.discarded_weight_max);
        for (total, sample) in self
            .discarded_weight_buckets
            .iter_mut()
            .zip(metrics.discarded_weight_buckets)
        {
            *total = total.saturating_add(sample);
        }
    }
}

/// Opt-in counters for performance work that crosses the live projection
/// boundary. The default is disabled so normal sessions only pay the branch
/// in the small recording methods.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PerformanceCounters {
    pub enabled: bool,

    pub stage_time_changes: u64,
    pub animation_runtime_samples: u64,
    pub animation_runtime_rebuilds: u64,
    pub animation_generic_patch_calls: u64,
    pub animation_usd_path_parses: u64,
    pub animation_read_mesh_calls: u64,
    pub animation_mesh_allocations: u64,
    pub animation_material_allocations: u64,

    pub projection_paths_planned: u64,
    pub projection_paths_materialized: u64,
    pub projection_full_stage_walks: u64,
    pub projection_subtree_walks: u64,

    pub reconcile_distinct_prims: u64,
    pub reconcile_changed_properties: u64,
    pub reconcile_dependency_queries: u64,
    pub reconcile_string_materializations: u64,

    pub semantic_snapshot_deep_clones: u64,
    pub scene_index_full_scans: u64,
    pub hierarchy_query_full_scans: u64,
}

impl PerformanceCounters {
    /// Clear measured values while preserving whether recording is enabled.
    pub fn reset(&mut self) {
        let enabled = self.enabled;
        *self = Self {
            enabled,
            ..Self::default()
        };
    }
}

macro_rules! counter_adders {
    ($($field:ident),+ $(,)?) => {
        $(
            impl PerformanceCounters {
                #[inline]
                pub fn $field(&mut self, count: u64) {
                    if self.enabled {
                        self.$field = self.$field.saturating_add(count);
                    }
                }
            }
        )+
    };
}

counter_adders!(
    stage_time_changes,
    animation_runtime_samples,
    animation_runtime_rebuilds,
    animation_generic_patch_calls,
    animation_usd_path_parses,
    animation_read_mesh_calls,
    animation_mesh_allocations,
    animation_material_allocations,
    projection_paths_planned,
    projection_paths_materialized,
    projection_full_stage_walks,
    projection_subtree_walks,
    reconcile_distinct_prims,
    reconcile_changed_properties,
    reconcile_dependency_queries,
    reconcile_string_materializations,
    semantic_snapshot_deep_clones,
    scene_index_full_scans,
    hierarchy_query_full_scans,
);

#[cfg(test)]
mod tests {
    use super::PerformanceCounters;

    #[test]
    fn disabled_counters_are_no_op() {
        let mut counters = PerformanceCounters::default();
        counters.stage_time_changes(3);
        assert_eq!(counters.stage_time_changes, 0);
    }

    #[test]
    fn reset_preserves_enabled_state() {
        let mut counters = PerformanceCounters {
            enabled: true,
            ..PerformanceCounters::default()
        };
        counters.stage_time_changes(4);
        counters.reset();
        assert!(counters.enabled);
        assert_eq!(counters.stage_time_changes, 0);
    }
}
