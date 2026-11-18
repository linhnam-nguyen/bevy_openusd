# BOV-C5 Scene Index and Hierarchy Test Migration Ledger

Mission `M-BOV-20261117-PC-CDS16-01`, plan revision
`sha256:3f7378770cdbafa638ef443f16e5a0cd60dee69fbaf938a3e6c8b6023c15c56b`,
checkpoint BOV-C5, iteration 1. The pre-change `bevy_openusd` commit was
`4e8fbe55e7b62f1ea4064f61a0c8eefc0e79d0b8` on `develop/refactoring`.

## Source path moves

| Before | After |
| --- | --- |
| `src/viewport/api/index/dense.rs` | `src/viewport/scene/index/dense.rs` |
| `src/viewport/api/index/hierarchy.rs` | `src/viewport/scene/index/hierarchy.rs` |
| `src/viewport/api/index/incremental.rs` | `src/viewport/scene/index/incremental.rs` |
| `src/viewport/api/index/lookup.rs` | `src/viewport/scene/index/lookup.rs` |
| `src/viewport/api/index/mod.rs` | `src/viewport/scene/index/mod.rs` |
| `src/viewport/api/index/occurrence.rs` | `src/viewport/scene/index/occurrence.rs` |
| `src/viewport/api/index/rebuild.rs` | `src/viewport/scene/index/rebuild.rs` |
| `src/viewport/api/index/reconcile.rs` | `src/viewport/scene/index/reconcile.rs` |
| `src/viewport/api/index/refresh.rs` | `src/viewport/scene/index/refresh.rs` |
| `src/viewport/api/hierarchy.rs` | `src/viewport/scene/hierarchy/mod.rs` |
| `src/viewport/api/hierarchy_visibility.rs` | `src/viewport/scene/hierarchy/visibility.rs` |

## External test and support path moves

| Before | After |
| --- | --- |
| `tests/unit/viewport/api/scene_index_tests.rs` | `tests/unit/viewport/scene/index/tests.rs` |
| `tests/unit/viewport/api/scene_index_incremental_tests.rs` | `tests/unit/viewport/scene/index/incremental_tests.rs` |
| `tests/unit/viewport/api/scene_index_lifecycle_tests.rs` | `tests/unit/viewport/scene/index/lifecycle_tests.rs` |
| `tests/unit/viewport/api/scene_index_reconcile_inline_tests.rs` | `tests/unit/viewport/scene/index/reconcile_tests.rs` |
| `tests/unit/viewport/api/scene_index_test_support.rs` | `tests/unit/viewport/scene/index/test_support.rs` |
| `tests/unit/viewport/api/bridge/tests/hierarchy_search_test.rs` | `tests/unit/viewport/api/bridge/tests/query/hierarchy_search_tests.rs` |
| `tests/unit/viewport/api/bridge/tests/hierarchy_visibility_test.rs` | `tests/unit/viewport/api/bridge/tests/query/hierarchy_visibility_tests.rs` |

The bridge test module names `hierarchy_search_test` and
`hierarchy_visibility_test` remain attached by their `#[path]` declarations.
Their test bodies and logical identities did not change. No alias is needed
for these two test modules.

## Exact scene-index identity migration

The BO-M9-C1 pinned name and test suffix are retained. This checkpoint updates
the existing BOR-C7 mapping's final destination from the C4 module path
`viewport::api::index` to `viewport::scene::index`; it does not add map rows or
change baseline data. Each logical identity is present in all six configured
platform/feature combinations and in both `Cargo.toml` targets (`usdview` bin
and lib), one occurrence per target. Therefore the exact application count is
32 target-qualified identities per configuration. The mapping remains direct
from the pinned source name to the final name; no alias chains are introduced.

| Pinned BO-M9-C1 / BOR-C7 source name | C4 destination | C5 destination |
| --- | --- | --- |
| `viewport::api::scene_index::lifecycle_tests::project_and_direct_scene_activation_keep_root_authority_through_refresh` | `viewport::api::index::lifecycle_tests::project_and_direct_scene_activation_keep_root_authority_through_refresh` | `viewport::scene::index::lifecycle_tests::project_and_direct_scene_activation_keep_root_authority_through_refresh` |
| `viewport::api::scene_index::reconcile::tests::dense_member_removal_repairs_the_swapped_position` | `viewport::api::index::reconcile::tests::dense_member_removal_repairs_the_swapped_position` | `viewport::scene::index::reconcile::tests::dense_member_removal_repairs_the_swapped_position` |
| `viewport::api::scene_index::tests::classification_visibility_reports_mixed_state_and_restores_groups` | `viewport::api::index::tests::classification_visibility_reports_mixed_state_and_restores_groups` | `viewport::scene::index::tests::classification_visibility_reports_mixed_state_and_restores_groups` |
| `viewport::api::scene_index::tests::dense_scene_index_pages_and_reveal_metadata_are_bounded` | `viewport::api::index::tests::dense_scene_index_pages_and_reveal_metadata_are_bounded` | `viewport::scene::index::tests::dense_scene_index_pages_and_reveal_metadata_are_bounded` |
| `viewport::api::scene_index::tests::generic_projection_keeps_snapshot_acquisition_constant_time` | `viewport::api::index::tests::generic_projection_keeps_snapshot_acquisition_constant_time` | `viewport::scene::index::tests::generic_projection_keeps_snapshot_acquisition_constant_time` |
| `viewport::api::scene_index::tests::hierarchy_snapshot_reuses_cached_projection` | `viewport::api::index::tests::hierarchy_snapshot_reuses_cached_projection` | `viewport::scene::index::tests::hierarchy_snapshot_reuses_cached_projection` |
| `viewport::api::scene_index::tests::incremental_tests::initial_authority_ingestion_is_bounded_and_converges_without_virtual_root_capacity` | `viewport::api::index::tests::incremental_tests::initial_authority_ingestion_is_bounded_and_converges_without_virtual_root_capacity` | `viewport::scene::index::tests::incremental_tests::initial_authority_ingestion_is_bounded_and_converges_without_virtual_root_capacity` |
| `viewport::api::scene_index::tests::incremental_tests::progressive_unique_prim_additions_use_incremental_index_ingestion` | `viewport::api::index::tests::incremental_tests::progressive_unique_prim_additions_use_incremental_index_ingestion` | `viewport::scene::index::tests::incremental_tests::progressive_unique_prim_additions_use_incremental_index_ingestion` |
| `viewport::api::scene_index::tests::incremental_tests::ten_thousand_prim_additions_are_admitted_with_a_hard_per_update_cap` | `viewport::api::index::tests::incremental_tests::ten_thousand_prim_additions_are_admitted_with_a_hard_per_update_cap` | `viewport::scene::index::tests::incremental_tests::ten_thousand_prim_additions_are_admitted_with_a_hard_per_update_cap` |
| `viewport::api::scene_index::tests::incremental_tests::ten_thousand_prim_full_reconciliation_capture_is_hard_capped` | `viewport::api::index::tests::incremental_tests::ten_thousand_prim_full_reconciliation_capture_is_hard_capped` | `viewport::scene::index::tests::incremental_tests::ten_thousand_prim_full_reconciliation_capture_is_hard_capped` |
| `viewport::api::scene_index::tests::native_instance_selection_resolves_real_projected_scene_proxy_paths_only` | `viewport::api::index::tests::native_instance_selection_resolves_real_projected_scene_proxy_paths_only` | `viewport::scene::index::tests::native_instance_selection_resolves_real_projected_scene_proxy_paths_only` |
| `viewport::api::scene_index::tests::prim_name_projection_uses_only_the_final_path_segment` | `viewport::api::index::tests::prim_name_projection_uses_only_the_final_path_segment` | `viewport::scene::index::tests::prim_name_projection_uses_only_the_final_path_segment` |
| `viewport::api::scene_index::tests::projection_only_children_churn_does_not_rebuild_the_scene_index` | `viewport::api::index::tests::projection_only_children_churn_does_not_rebuild_the_scene_index` | `viewport::scene::index::tests::projection_only_children_churn_does_not_rebuild_the_scene_index` |
| `viewport::api::scene_index::tests::semantic_path_resolution_preserves_runtime_reveal_pages` | `viewport::api::index::tests::semantic_path_resolution_preserves_runtime_reveal_pages` | `viewport::scene::index::tests::semantic_path_resolution_preserves_runtime_reveal_pages` |
| `viewport::api::scene_index::tests::semantic_path_visibility_targets_all_native_occurrences` | `viewport::api::index::tests::semantic_path_visibility_targets_all_native_occurrences` | `viewport::scene::index::tests::semantic_path_visibility_targets_all_native_occurrences` |
| `viewport::api::scene_index::tests::shared_provider_projection_installs_arc_and_matching_page_index` | `viewport::api::index::tests::shared_provider_projection_installs_arc_and_matching_page_index` | `viewport::scene::index::tests::shared_provider_projection_installs_arc_and_matching_page_index` |

Configuration scope for every row: `linux:default`, `linux:solari`,
`macos:default`, `macos:solari`, `windows:default`, `windows:solari`.
Every target scope is manifest `Cargo.toml`, target `usdview`, kind `bin` or
`lib`. Counts, target assignments and feature applicability remain pinned.

## Ownership, policy, and evidence

- The index and hierarchy modules now live under the scene owner. `api/mod.rs`
  drops the prior declarations and re-exports; `scene/mod.rs` declares the
  modules and preserves the internal scene-facing exports. Moved-source test
  attachments and the four affected reviewed-path policy records point at the
  new paths while retaining their module names.
- Consumer imports now refer to the scene-owned types. The bridge plugin keeps
  the same refresh system, ordering constraints and observer registrations;
  only the function's module path changes. Serialized data, protocol fields,
  `Cargo.toml` targets, features and CI selectors are unchanged. No
  crate-external public API was added. `UsdHubUI` remained read-only.
- Existing baselines were not regenerated or edited. The migration map remains
  at 294 distinct pinned source names; 16 rows carry BOV-C5 evidence and all
  16 destinations now point directly to `viewport::scene::index`. The map's
  strict exact target/configuration, collision, chain and multiplicity checks
  remain enabled.
- The added parity regression was observed RED before the C5 map update
  (expected 16 C5 destinations, found 0), then GREEN afterward (one focused
  test passed). The existing C4 I2 audit found generic parity CI invocations,
  with no C5-specific test-path selector to update.
- Codebase principles remain `UNCONFIGURED`; the active mission state marks
  them not applicable. This does not grant a behavior change.

Configured Compile and final source fingerprint will be recorded in the
authoritative working plan and Local Agent handoff after the assigned Compile
runner finishes. The Implementer does not run the formal Test layer, live
parity, checkpoint acceptance, commit, or push.
