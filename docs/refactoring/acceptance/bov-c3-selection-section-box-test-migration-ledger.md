# BOV-C3 Selection and Section Box Migration Ledger

Mission: M-BOV-20261117-PC-CDS16-01  
Plan revision: sha256:3f7378770cdbafa638ef443f16e5a0cd60dee69fbaf938a3e6c8b6023c15c56b  
Checkpoint / iteration: BOV-C3 / 1  
Pre-change `bevy_openusd` HEAD: 6b5dcfe9b70d4bfa73ef6cc7084fa6a48497e51b

This successor record covers BOV-C3 only. The BOV-C1 and BOV-C2 migration ledgers, older acceptance records, and enumeration baselines remain unchanged.

## Scope and runtime ownership

Implemented phase 3 of the Owner-selected BOV plan: extracted selection and section-box runtime ownership from `viewport::scene` into `viewport::selection` and `viewport::section_box`, and relocated the corresponding external viewport unit-test/support files in the same checkpoint.

`viewport::scene::entity_order` remains in the scene owner and is re-exported within `viewport` for the extracted selection feature. `viewport::scene::visualization` remains in scene and is consumed by the extracted section-box clipping feature. `viewport::residency::selection` remains in residency; it serves geometry residency demand and is distinct from selecting an object. Runtime code, schedules, feature gates, state contracts, and test bodies were not intentionally changed.

## Production source path mapping

| Previous path | BOV-C3 path |
| --- | --- |
| `src/viewport/scene/selection/mod.rs` | `src/viewport/selection/mod.rs` |
| `src/viewport/scene/selection/color/mod.rs` | `src/viewport/selection/color/mod.rs` |
| `src/viewport/scene/selection/color/apply.rs` | `src/viewport/selection/color/apply.rs` |
| `src/viewport/scene/selection/hover.rs` | `src/viewport/selection/hover.rs` |
| `src/viewport/scene/selection/outline/mod.rs` | `src/viewport/selection/outline/mod.rs` |
| `src/viewport/scene/selection/outline/apply.rs` | `src/viewport/selection/outline/apply.rs` |
| `src/viewport/scene/selection/outline/tree.rs` | `src/viewport/selection/outline/tree.rs` |
| `src/viewport/scene/selection/presentation/mod.rs` | `src/viewport/selection/presentation/mod.rs` |
| `src/viewport/scene/selection/presentation/proxy.rs` | `src/viewport/selection/presentation/proxy.rs` |
| `src/viewport/scene/selection/projection/mod.rs` | `src/viewport/selection/projection/mod.rs` |
| `src/viewport/scene/selection/projection/bounds.rs` | `src/viewport/selection/projection/bounds.rs` |
| `src/viewport/scene/selection/projection/membership.rs` | `src/viewport/selection/projection/membership.rs` |
| `src/viewport/scene/selection/projection/topology.rs` | `src/viewport/selection/projection/topology.rs` |
| `src/viewport/scene/section_box/mod.rs` | `src/viewport/section_box/mod.rs` |
| `src/viewport/scene/section_box/bounds.rs` | `src/viewport/section_box/bounds.rs` |
| `src/viewport/scene/section_box/face.rs` | `src/viewport/section_box/face.rs` |
| `src/viewport/scene/section_box/gizmo.rs` | `src/viewport/section_box/gizmo.rs` |
| `src/viewport/scene/section_box/pose.rs` | `src/viewport/section_box/pose.rs` |
| `src/viewport/scene/section_box/tracking.rs` | `src/viewport/section_box/tracking.rs` |
| `src/viewport/scene/section_box/visualization.rs` | `src/viewport/section_box/visualization.rs` |
| `src/viewport/scene/section_box/clipping/mod.rs` | `src/viewport/section_box/clipping/mod.rs` |
| `src/viewport/scene/section_box/clipping/support.rs` | `src/viewport/section_box/clipping/support.rs` |
| `src/viewport/scene/section_box/clipping/sync.rs` | `src/viewport/section_box/clipping/sync.rs` |

The old `scene::selection` and `scene::section_box` module declarations were removed. The viewport root now declares both feature owners with `pub(in crate::viewport)` visibility. `scene/mod.rs` re-exports the existing selection and section-box runtime items, preserving current scene-facing consumers. Internal feature imports now point to the selection owner or the scene visualization owner according to responsibility.

`EntityOrder` and its operations changed from `pub(super)` to `pub(in crate::viewport)` and are re-exported from `scene` at that same viewport scope. This is required because color, outline, and selection-projection code now live in a sibling feature owner while `EntityOrder` remains scene-owned. `HoveredTarget::anchor` changed from `pub(in crate::viewport::scene)` to `pub(in crate::viewport)` because the selection-owned type can no longer declare a sibling scene module as its visibility boundary. Both visibility changes expose only viewport-internal implementation details; they do not add a crate-external public API.

The configured Compile also exposed relocation-dependent references beyond the moved test attachments. The clipping module's two embedded shader paths now traverse four parents from its new location. The retained `scene/classification_color` and `scene/visualization` modules, plus `visualization/render_mode`, now import the moved feature types from `crate::viewport::section_box` and `crate::viewport::selection`; the moved section-box state system imports `SelectedTargets` from its new selection owner. These edits preserve the existing runtime relationships and do not expose new crate-external APIs.

## External test and support path mapping

| Previous path | BOV-C3 path | Source owner attachment |
| --- | --- | --- |
| `tests/unit/viewport/scene/section_box_clipping_tests.rs` | `tests/unit/viewport/section_box/clipping_tests.rs` | `src/viewport/section_box/clipping/mod.rs` |
| `tests/unit/viewport/scene/section_box_face_tests.rs` | `tests/unit/viewport/section_box/face_tests.rs` | `src/viewport/section_box/face.rs` |
| `tests/unit/viewport/scene/section_box_gizmo_inline_tests.rs` | `tests/unit/viewport/section_box/gizmo_tests.rs` | `src/viewport/section_box/gizmo.rs` |
| `tests/unit/viewport/scene/section_box_tests.rs` | `tests/unit/viewport/section_box/tests.rs` | `src/viewport/section_box/mod.rs` |
| `tests/unit/viewport/scene/section_box_visualization_inline_tests.rs` | `tests/unit/viewport/section_box/visualization_tests.rs` | `src/viewport/section_box/visualization.rs` |
| `tests/unit/viewport/scene/selection_color_inline_tests.rs` | `tests/unit/viewport/selection/color_tests.rs` | `src/viewport/selection/color/mod.rs` |
| `tests/unit/viewport/scene/selection_hover_inline_tests.rs` | `tests/unit/viewport/selection/hover_tests.rs` | `src/viewport/selection/hover.rs` |
| `tests/unit/viewport/scene/selection_outline_inline_tests.rs` | `tests/unit/viewport/selection/outline_tests.rs` | `src/viewport/selection/outline/mod.rs` |
| `tests/unit/viewport/scene/selection_presentation_proxy_inline_tests.rs` | `tests/unit/viewport/selection/presentation/proxy_tests.rs` | `src/viewport/selection/presentation/proxy.rs` |
| `tests/unit/viewport/scene/selection_tests.rs` | `tests/unit/viewport/selection/tests.rs` | `src/viewport/selection/mod.rs` |
| `tests/unit/viewport/scene/selection_projection_test_support.rs` | `tests/unit/viewport/selection/projection/test_support.rs` | `src/viewport/selection/projection/mod.rs` |

All 11 `#[path]` declarations retain their `mod_name` and empty `module_path`; only the physical target paths changed. The ten test files and one support file were moved without body edits. `scripts/test_architecture_policy.json` was updated for all 11 owner/target pairs. Its approved production dependency record for section-box clipping support now names the new support, owner, and consumer paths. No `Cargo.toml` target registration or feature gate changed.

## Test identity correspondence and expected counts

The pre-C3 source fingerprint was `4b9940311571574a53fdd9396980b7938ad3edbcf13395055ef9ad9be31617ee` in the C2 live parity receipt. That receipt records 1,966 target-qualified identities, with zero missing/unexpected identities and all 18 C2 ledger identities present. The C2 live target listing matched the BOR-C7 captured current-target inventory. From that verified pre-C3 listing, the 42 affected selection/section-box identities map as follows; every listed function suffix is retained exactly once.

| Previous target-qualified prefix | BOV-C3 target-qualified prefix | Test function identities retained | Count |
| --- | --- | --- | ---: |
| `viewport::scene::section_box::clipping::tests` | `viewport::section_box::clipping::tests` | `clip_uniform_uses_one_shared_box_space_transform`; `composition_preserves_uniform_and_selection_routes`; `composition_restores_authored_route_after_shaded_selection_release`; `extension_keeps_the_standard_material_as_the_base_route`; `stale_uniform_route_is_reconciled_before_shaded_clip_removal`; `successful_clip_clears_recovered_missing_material_diagnostic`; `successful_clip_clears_recovered_unsupported_diagnostic` | 7 |
| `viewport::scene::section_box::face::tests` | `viewport::section_box::face::tests` | `every_face_contracts_and_extends_without_moving_other_faces`; `minimum_thickness_clamps_crossing_and_uses_effective_delta`; `repeated_incremental_drag_and_reverse_restore_the_original_pose`; `rotated_face_drag_uses_local_normal_for_all_axis_pairs` | 4 |
| `viewport::scene::section_box::gizmo::tests` | `viewport::section_box::gizmo::tests` | `face_adjusted_box_preserves_dimensions_through_translation_and_rotation`; `face_drag_promotes_autofit_to_user_adjusted_and_preserves_drag_baseline`; `visible_state_spawns_exactly_one_aggregate_gizmo_target` | 3 |
| `viewport::scene::section_box::tests` | `viewport::section_box::tests` | `aggregate_bounds_contain_every_selected_renderable`; `direct_selection_changes_advance_bounds_generation_without_disabling_section_box`; `disabling_resets_user_adjustment_and_reenable_refits`; `empty_geometry_resets_derived_state_without_authored_geometry`; `fit_produces_one_box_transform_and_six_planes`; `material_only_component_change_does_not_reconcile_section_box`; `oriented_transform_produces_six_inside_facing_planes`; `selection_change_refits_user_adjusted_pose`; `unrelated_changes_do_not_require_section_box_reconciliation`; `user_adjusted_pose_survives_reference_bounds_update`; `user_manipulation_preserves_bounds_generation` | 11 |
| `viewport::scene::section_box::visualization::tests` | `viewport::section_box::visualization::tests` | `aggregate_visualization_has_exactly_one_box`; `visualization_follows_the_interactive_box_transform` | 2 |
| `viewport::scene::selection::color::tests` | `viewport::selection::color::tests` | `default_selection_color_override_is_disabled_but_has_documented_color`; `selection_color_has_priority_over_hover_color`; `selection_color_preserves_rgb_channels` | 3 |
| `viewport::scene::selection::hover::tests` | `viewport::selection::hover::tests` | `hover_pick_stats_start_without_raycasts`; `idle_pick_state_requires_no_new_input_or_camera_revision`; `remote_css_pointer_maps_non_two_x_camera_viewport`; `remote_css_pointer_maps_to_actual_camera_viewport` | 4 |
| `viewport::scene::selection::outline::tests` | `viewport::selection::outline::tests` | `default_selection_boundary_uses_the_documented_width`; `selection_outline_color_preserves_rgb_channels` | 2 |
| `viewport::scene::selection::presentation::proxy::tests` | `viewport::selection::presentation::proxy::tests` | `coarse_proxy_has_one_wire_box` | 1 |
| `viewport::scene::selection::tests` | `viewport::selection::tests` | `large_selection_transactions_advance_the_authority_once_each`; `logical_selection_is_canonical_and_primary_remains_a_member`; `logical_selection_rejects_duplicates_before_state_mutation`; `pending_selection_delta_coalesces_until_projection_consumes_it`; `selected_instance_bridge_captures_identity_before_reconcile` | 5 |
| **Total** |  |  | **42** |

The target-qualified change is solely removal of the former `scene` owner segment. Test-module names and test function names stay the same. The test enumeration baselines were not regenerated or edited. The projection support attachment contributes no test identity. The Implementer did not run formal Tests or live enumeration; Local Agent retains those gates and must compare the post-C3 listing against this 42-name correspondence.

## Policy, scope, and limitations

- The active phase-3 source/test mappings are implemented for 23 production files and 11 external test/support files.
- `scripts/test_architecture_policy.json` attachment and production-owner paths were updated; historical policy rationale is retained.
- `scripts/test_enumeration_qualified_baseline.json` and `scripts/test_enumeration_baseline.json` were not changed.
- The historical backend test census and BOV-C1/BOV-C2 ledgers remain unchanged.
- `residency/selection.rs`, UsdHubUI, Cargo target registrations, CI workflows, and later-phase viewport owners were not modified.
- Runtime behavior and test bodies were preserved by source/test file relocation; configured Compile and the architecture guard are still required to verify the final source.
- The first configured Compile stopped at `cargo fmt --all -- --check`: all 11 moved-owner test `#[path]` attributes still traversed one extra parent directory after the source owner directories moved out of `scene/`. They were corrected by reducing each relative path by one `..`. An independent resolved-target check confirmed all 11 now resolve to their exact mapped files and exist.
- The next configured Compile reached the formatter and reported only import-order/blank-line differences in six edited Rust files. Applied `cargo fmt --all`; the scoped path check found no out-of-scope product changes.
- The following configured Compile passed formatting and stopped at the workspace build with 12 errors: two stale embedded shader paths, references through the former `scene` siblings/facade, and one `SelectedTargets` owner path. The `state.targets` mismatch disappeared after importing `SelectedTargets` from its new owner, confirming it was a cascading diagnostic. The failed attempt and diagnosis are recorded in the active working plan; the canonical Compile log contains the final retry.
- Final configured Compile: **PASS** via Python 3.14 `Validation.py --layer compile`; receipt `AgentsComm/communication/agents/compile.log`. The active working plan records the receipt-bound source fingerprint. All five configured commands passed, including the test architecture guard with `violation_count: 0`.
- Formal Test and live target parity were not run by the Implementer, as required by the role assignment; Local Agent retains those gates.
