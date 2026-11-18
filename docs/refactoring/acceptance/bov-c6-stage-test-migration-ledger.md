# BOV-C6 Stage Lifecycle and Test Migration Ledger

Mission `M-BOV-20261117-PC-CDS16-01`; plan revision `sha256:3f7378770cdbafa638ef443f16e5a0cd60dee69fbaf938a3e6c8b6023c15c56b`; checkpoint BOV-C6, iteration 1.

Writable source: `bevy_openusd`, branch `develop/refactoring`, pre-change HEAD `a90c2db623d5052e3224be5f350de3e5ed0592b3` and pre-change source fingerprint `f778e2c73bac479bd870c7fcd8cbb2214a0be06a177bc5db2dfd14a79b60d2c9`. The pinned BO-M9-C1 identity baseline remains at `59ccddbea7be85a34a8e8f1d4d131c91c7ade858`.

## Scope and source moves

The active USD stage lifecycle moved from `viewport/session` to `viewport/stage`; viewport Project activation and LiveStage runtime ownership moved under `viewport/stage/project`. Protocol client registration in `api/session_registry.rs`, network/video transport connections, and application-level `project/stages/` operations stay in their existing owners.

`app/project/stage.rs` was inspected before naming. It owns the render-host Project-to-LiveStage mutation handoff, runtime authority lease checks, and lease expiry. It is now `stage/project/live_stage_runtime.rs`; the companion test file is `live_stage_runtime_tests.rs`.

| Source path | C6 destination |
| --- | --- |
| `src/viewport/app/project/activation/flow.rs` | `src/viewport/stage/project/activation/flow.rs` |
| `src/viewport/app/project/activation/mod.rs` | `src/viewport/stage/project/activation/mod.rs` |
| `src/viewport/app/project/activation/queue.rs` | `src/viewport/stage/project/activation/queue.rs` |
| `src/viewport/app/project/activation/support.rs` | `src/viewport/stage/project/activation/support.rs` |
| `src/viewport/app/project/mod.rs` | `src/viewport/stage/project/mod.rs` |
| `src/viewport/app/project/runtime_authority.rs` | `src/viewport/stage/project/runtime_authority.rs` |
| `src/viewport/app/project/runtime_snapshot.rs` | `src/viewport/stage/project/runtime_snapshot.rs` |
| `src/viewport/app/project/stage.rs` | `src/viewport/stage/project/live_stage_runtime.rs` |
| `src/viewport/session/cache_presentation_gate.rs` | `src/viewport/stage/cache_presentation_gate.rs` |
| `src/viewport/session/lifecycle/invalidation.rs` | `src/viewport/stage/lifecycle/invalidation.rs` |
| `src/viewport/session/lifecycle/metadata.rs` | `src/viewport/stage/lifecycle/metadata.rs` |
| `src/viewport/session/lifecycle/mod.rs` | `src/viewport/stage/lifecycle/mod.rs` |
| `src/viewport/session/lifecycle/open.rs` | `src/viewport/stage/lifecycle/open.rs` |
| `src/viewport/session/lifecycle/prim_count.rs` | `src/viewport/stage/lifecycle/prim_count.rs` |
| `src/viewport/session/lifecycle/project_activation.rs` | `src/viewport/stage/lifecycle/project_activation.rs` |
| `src/viewport/session/lifecycle/scene_cache.rs` | `src/viewport/stage/lifecycle/scene_cache.rs` |
| `src/viewport/session/lifecycle/scene_presentation.rs` | `src/viewport/stage/lifecycle/scene_presentation.rs` |
| `src/viewport/session/mod.rs` | `src/viewport/stage/mod.rs` |
| `src/viewport/session/state.rs` | `src/viewport/stage/state.rs` |

## External test and support moves

| Source path | C6 destination |
| --- | --- |
| `tests/unit/viewport/app/project_activation_cache_first_tests.rs` | `tests/unit/viewport/stage/project/activation/cache_first_tests.rs` |
| `tests/unit/viewport/app/project_activation_cache_tests.rs` | `tests/unit/viewport/stage/project/activation/cache_tests.rs` |
| `tests/unit/viewport/app/project_activation_flow_test_support.rs` | `tests/unit/viewport/stage/project/activation/flow_test_support.rs` |
| `tests/unit/viewport/app/project_activation_inline_tests.rs` | `tests/unit/viewport/stage/project/activation/tests.rs` |
| `tests/unit/viewport/app/project_activation_production_tests.rs` | `tests/unit/viewport/stage/project/activation/production_tests.rs` |
| `tests/unit/viewport/app/project_activation_queue_inline_tests.rs` | `tests/unit/viewport/stage/project/activation/queue_tests.rs` |
| `tests/unit/viewport/app/project_activation_test_support.rs` | `tests/unit/viewport/stage/project/activation/test_support.rs` |
| `tests/unit/viewport/app/project_runtime_authority_inline_tests.rs` | `tests/unit/viewport/stage/project/runtime_authority_tests.rs` |
| `tests/unit/viewport/app/project_runtime_authority_lease_tests.rs` | `tests/unit/viewport/stage/project/runtime_authority_lease_tests.rs` |
| `tests/unit/viewport/app/project_runtime_snapshot_test_support.rs` | `tests/unit/viewport/stage/project/runtime_snapshot_test_support.rs` |
| `tests/unit/viewport/app/project_stage_tests.rs` | `tests/unit/viewport/stage/project/live_stage_runtime_tests.rs` |
| `tests/unit/viewport/session/cache_presentation_gate_inline_tests.rs` | `tests/unit/viewport/stage/cache_presentation_gate_tests.rs` |
| `tests/unit/viewport/session/cache_presentation_gate_test_support.rs` | `tests/unit/viewport/stage/cache_presentation_gate_test_support.rs` |
| `tests/unit/viewport/session/lifecycle_cache_preservation_tests.rs` | `tests/unit/viewport/stage/lifecycle/cache_preservation_tests.rs` |
| `tests/unit/viewport/session/lifecycle_cache_tests.rs` | `tests/unit/viewport/stage/lifecycle/cache_tests.rs` |
| `tests/unit/viewport/session/lifecycle_inline_tests.rs` | `tests/unit/viewport/stage/lifecycle/tests.rs` |
| `tests/unit/viewport/session/lifecycle_prim_count_tests.rs` | `tests/unit/viewport/stage/lifecycle/prim_count_tests.rs` |
| `tests/unit/viewport/session/lifecycle_scene_presentation_inline_tests.rs` | `tests/unit/viewport/stage/lifecycle/scene_presentation_tests.rs` |
| `tests/unit/viewport/session/lifecycle_test_support.rs` | `tests/unit/viewport/stage/lifecycle/test_support.rs` |

The 19 source files and 19 external test/support files were relocated without changing their behavior. Only imports, `#[path]` attachments, module wiring, and the two nested test-file includes were adjusted. No dormant tests were activated.

## Exact identity migration

Identity source is the unchanged BO-M9-C1 target-qualified baseline. Each row shows its pinned source identity, the pre-C6 C5 destination, and the C6 destination. All rows apply to `linux:default`, `linux:solari`, `macos:default`, `macos:solari`, `windows:default`, and `windows:solari`, on `Cargo.toml` target `usdview` of kinds `bin` and `lib` (one occurrence in each target).

| Pinned BO-M9-C1 identity | Pre-C6 C5 identity | C6 identity | Listing kind |
| --- | --- | --- | --- |
| `viewport::session::cache_presentation_gate::tests::a_stale_rendered_generation_does_not_release_the_gate` | `viewport::session::cache_presentation_gate::tests::a_stale_rendered_generation_does_not_release_the_gate` | `viewport::stage::cache_presentation_gate::tests::a_stale_rendered_generation_does_not_release_the_gate` | `test` |
| `viewport::session::cache_presentation_gate::tests::rendered_frame_opens_immediately_but_missing_renderer_uses_bounded_fallback` | `viewport::session::cache_presentation_gate::tests::rendered_frame_opens_immediately_but_missing_renderer_uses_bounded_fallback` | `viewport::stage::cache_presentation_gate::tests::rendered_frame_opens_immediately_but_missing_renderer_uses_bounded_fallback` | `test` |
| `viewport::session::lifecycle::cache_tests::changed_source_across_stage_open_cannot_consume_old_cache_seeds` | `viewport::session::lifecycle::cache_tests::changed_source_across_stage_open_cannot_consume_old_cache_seeds` | `viewport::stage::lifecycle::cache_tests::changed_source_across_stage_open_cannot_consume_old_cache_seeds` | `test` |
| `viewport::session::lifecycle::cache_tests::corrupt_cache_falls_back_to_a_successfully_opened_canonical_stage` | `viewport::session::lifecycle::cache_tests::corrupt_cache_falls_back_to_a_successfully_opened_canonical_stage` | `viewport::stage::lifecycle::cache_tests::corrupt_cache_falls_back_to_a_successfully_opened_canonical_stage` | `test` |
| `viewport::session::lifecycle::cache_tests::poisoned_scene_revalidation_discards_cache_presentation_without_losing_live_stage` | `viewport::session::lifecycle::cache_tests::poisoned_scene_revalidation_discards_cache_presentation_without_losing_live_stage` | `viewport::stage::lifecycle::cache_tests::poisoned_scene_revalidation_discards_cache_presentation_without_losing_live_stage` | `test` |
| `viewport::session::lifecycle::cache_tests::preservation_tests::canonical_stage_install_preserves_cache_presentation_lookup_and_resident_meshes` | `viewport::session::lifecycle::cache_tests::preservation_tests::canonical_stage_install_preserves_cache_presentation_lookup_and_resident_meshes` | `viewport::stage::lifecycle::cache_tests::preservation_tests::canonical_stage_install_preserves_cache_presentation_lookup_and_resident_meshes` | `test` |
| `viewport::session::lifecycle::cache_tests::unequal_activation_and_scene_generations_revalidate_current_scene_cache` | `viewport::session::lifecycle::cache_tests::unequal_activation_and_scene_generations_revalidate_current_scene_cache` | `viewport::stage::lifecycle::cache_tests::unequal_activation_and_scene_generations_revalidate_current_scene_cache` | `test` |
| `viewport::session::lifecycle::prim_count::tests::descriptor_appearing_after_uncached_activation_reconciles_same_session` | `viewport::session::lifecycle::prim_count::tests::descriptor_appearing_after_uncached_activation_reconciles_same_session` | `viewport::stage::lifecycle::prim_count::tests::descriptor_appearing_after_uncached_activation_reconciles_same_session` | `test` |
| `viewport::session::lifecycle::prim_count::tests::late_prior_session_result_cannot_mutate_current_lifecycle` | `viewport::session::lifecycle::prim_count::tests::late_prior_session_result_cannot_mutate_current_lifecycle` | `viewport::stage::lifecycle::prim_count::tests::late_prior_session_result_cannot_mutate_current_lifecycle` | `test` |
| `viewport::session::lifecycle::prim_count::tests::malformed_index_publication_is_retryable_lifecycle_failure` | `viewport::session::lifecycle::prim_count::tests::malformed_index_publication_is_retryable_lifecycle_failure` | `viewport::stage::lifecycle::prim_count::tests::malformed_index_publication_is_retryable_lifecycle_failure` | `test` |
| `viewport::session::lifecycle::prim_count::tests::malformed_spatial_publication_is_retryable_lifecycle_failure` | `viewport::session::lifecycle::prim_count::tests::malformed_spatial_publication_is_retryable_lifecycle_failure` | `viewport::stage::lifecycle::prim_count::tests::malformed_spatial_publication_is_retryable_lifecycle_failure` | `test` |
| `viewport::session::lifecycle::prim_count::tests::newer_generation_rejection_leaves_lifecycle_metadata_unchanged` | `viewport::session::lifecycle::prim_count::tests::newer_generation_rejection_leaves_lifecycle_metadata_unchanged` | `viewport::stage::lifecycle::prim_count::tests::newer_generation_rejection_leaves_lifecycle_metadata_unchanged` | `test` |
| `viewport::session::lifecycle::prim_count::tests::ordinary_non_project_activation_has_no_cache_write` | `viewport::session::lifecycle::prim_count::tests::ordinary_non_project_activation_has_no_cache_write` | `viewport::stage::lifecycle::prim_count::tests::ordinary_non_project_activation_has_no_cache_write` | `test` |
| `viewport::session::lifecycle::prim_count::tests::prim_count_retries_have_a_terminal_bound` | `viewport::session::lifecycle::prim_count::tests::prim_count_retries_have_a_terminal_bound` | `viewport::stage::lifecycle::prim_count::tests::prim_count_retries_have_a_terminal_bound` | `test` |
| `viewport::session::lifecycle::prim_count::tests::same_generation_merge_preserves_latest_cache_metadata` | `viewport::session::lifecycle::prim_count::tests::same_generation_merge_preserves_latest_cache_metadata` | `viewport::stage::lifecycle::prim_count::tests::same_generation_merge_preserves_latest_cache_metadata` | `test` |
| `viewport::session::lifecycle::project_activation::scene_presentation::tests::child_flags_are_computed_in_one_parent_index_pass` | `viewport::session::lifecycle::scene_presentation::tests::child_flags_are_computed_in_one_parent_index_pass` | `viewport::stage::lifecycle::scene_presentation::tests::child_flags_are_computed_in_one_parent_index_pass` | `test` |
| `viewport::session::lifecycle::tests::project_activation_replaces_a_with_b_and_failed_b_preserves_a` | `viewport::session::lifecycle::tests::project_activation_replaces_a_with_b_and_failed_b_preserves_a` | `viewport::stage::lifecycle::tests::project_activation_replaces_a_with_b_and_failed_b_preserves_a` | `test` |
| `viewport::app::project_activation::cache_tests::activation_does_not_wait_for_inflight_cache_warm` | `viewport::app::project::activation::cache_tests::activation_does_not_wait_for_inflight_cache_warm` | `viewport::stage::project::activation::cache_tests::activation_does_not_wait_for_inflight_cache_warm` | `test` |
| `viewport::app::project_activation::cache_tests::real_project_hummingbird_activation_reaches_geometry_ready_and_playback` | `viewport::app::project::activation::cache_tests::real_project_hummingbird_activation_reaches_geometry_ready_and_playback` | `viewport::stage::project::activation::cache_tests::real_project_hummingbird_activation_reaches_geometry_ready_and_playback` | `test` |
| `viewport::app::project_activation::production_tests::cache_first_tests::scene_cache_presentation_is_published_before_blocking_stage_open` | `viewport::app::project::activation::production_tests::cache_first_tests::scene_cache_presentation_is_published_before_blocking_stage_open` | `viewport::stage::project::activation::production_tests::cache_first_tests::scene_cache_presentation_is_published_before_blocking_stage_open` | `test` |
| `viewport::app::project_activation::production_tests::cache_first_tests::stale_cache_first_continuation_rolls_back_without_replacing_newer_authority` | `viewport::app::project::activation::production_tests::cache_first_tests::stale_cache_first_continuation_rolls_back_without_replacing_newer_authority` | `viewport::stage::project::activation::production_tests::cache_first_tests::stale_cache_first_continuation_rolls_back_without_replacing_newer_authority` | `test` |
| `viewport::app::project_activation::production_tests::production_activation_keeps_live_semantic_bim_and_provider_state_coherent` | `viewport::app::project::activation::production_tests::production_activation_keeps_live_semantic_bim_and_provider_state_coherent` | `viewport::stage::project::activation::production_tests::production_activation_keeps_live_semantic_bim_and_provider_state_coherent` | `test` |
| `viewport::app::project_activation::queue::tests::blocked_preparation_accepts_newest_request_without_waiting_for_old_one` | `viewport::app::project::activation::queue::tests::blocked_preparation_accepts_newest_request_without_waiting_for_old_one` | `viewport::stage::project::activation::queue::tests::blocked_preparation_accepts_newest_request_without_waiting_for_old_one` | `test` |
| `viewport::app::project_activation::queue::tests::latest_request_replaces_blocked_preparation` | `viewport::app::project::activation::queue::tests::latest_request_replaces_blocked_preparation` | `viewport::stage::project::activation::queue::tests::latest_request_replaces_blocked_preparation` | `test` |
| `viewport::app::project_activation::queue::tests::superseded_request_is_not_current` | `viewport::app::project::activation::queue::tests::superseded_request_is_not_current` | `viewport::stage::project::activation::queue::tests::superseded_request_is_not_current` | `test` |
| `viewport::app::project_activation::tests::activation_reply_preserves_project_root_generation_and_has_no_path` | `viewport::app::project::activation::tests::activation_reply_preserves_project_root_generation_and_has_no_path` | `viewport::stage::project::activation::tests::activation_reply_preserves_project_root_generation_and_has_no_path` | `test` |
| `viewport::app::project_activation::tests::activation_requests_are_routed_to_the_requesting_session` | `viewport::app::project::activation::tests::activation_requests_are_routed_to_the_requesting_session` | `viewport::stage::project::activation::tests::activation_requests_are_routed_to_the_requesting_session` | `test` |
| `viewport::app::project_activation::tests::preparation_worker_preserves_request_identity_when_registry_is_unavailable` | `viewport::app::project::activation::tests::preparation_worker_preserves_request_identity_when_registry_is_unavailable` | `viewport::stage::project::activation::tests::preparation_worker_preserves_request_identity_when_registry_is_unavailable` | `test` |
| `viewport::app::project_stage::tests::active_scene_identity_requires_the_canonical_scene_locator` | `viewport::app::project::stage::tests::active_scene_identity_requires_the_canonical_scene_locator` | `viewport::stage::project::live_stage_runtime::tests::active_scene_identity_requires_the_canonical_scene_locator` | `test` |
| `viewport::app::project_stage::tests::project_root_is_derived_only_for_a_project_scene_path` | `viewport::app::project::stage::tests::project_root_is_derived_only_for_a_project_scene_path` | `viewport::stage::project::live_stage_runtime::tests::project_root_is_derived_only_for_a_project_scene_path` | `test` |
| `viewport::app::project_stage::project_runtime_authority::lease_tests::active_runtime_stage_returns_ready_and_freezes_authoring` | `viewport::app::project::runtime_authority::lease_tests::active_runtime_stage_returns_ready_and_freezes_authoring` | `viewport::stage::project::runtime_authority::lease_tests::active_runtime_stage_returns_ready_and_freezes_authoring` | `test` |
| `viewport::app::project_stage::project_runtime_authority::lease_tests::empty_inbox_skips_registry_resolution_for_any_project_count` | `viewport::app::project::runtime_authority::lease_tests::empty_inbox_skips_registry_resolution_for_any_project_count` | `viewport::stage::project::runtime_authority::lease_tests::empty_inbox_skips_registry_resolution_for_any_project_count` | `test` |
| `viewport::app::project_stage::project_runtime_authority::lease_tests::healthy_runtime_lease_renewal_extends_deadline_and_stays_frozen` | `viewport::app::project::runtime_authority::lease_tests::healthy_runtime_lease_renewal_extends_deadline_and_stays_frozen` | `viewport::stage::project::runtime_authority::lease_tests::healthy_runtime_lease_renewal_extends_deadline_and_stays_frozen` | `test` |
| `viewport::app::project_stage::project_runtime_authority::lease_tests::timed_out_consumed_request_never_freezes_or_creates_a_lease` | `viewport::app::project::runtime_authority::lease_tests::timed_out_consumed_request_never_freezes_or_creates_a_lease` | `viewport::stage::project::runtime_authority::lease_tests::timed_out_consumed_request_never_freezes_or_creates_a_lease` | `test` |
| `viewport::app::project_stage::project_runtime_authority::lease_tests::timeout_between_export_and_lease_claim_cannot_publish_ready` | `viewport::app::project::runtime_authority::lease_tests::timeout_between_export_and_lease_claim_cannot_publish_ready` | `viewport::stage::project::runtime_authority::lease_tests::timeout_between_export_and_lease_claim_cannot_publish_ready` | `test` |
| `viewport::app::project_stage::project_runtime_authority::tests::expired_runtime_lease_unfreezes_live_stage` | `viewport::app::project::runtime_authority::tests::expired_runtime_lease_unfreezes_live_stage` | `viewport::stage::project::runtime_authority::tests::expired_runtime_lease_unfreezes_live_stage` | `test` |
| `viewport::app::project_stage::project_runtime_authority::tests::no_active_stage_answers_registered_requests_as_inactive` | `viewport::app::project::runtime_authority::tests::no_active_stage_answers_registered_requests_as_inactive` | `viewport::stage::project::runtime_authority::tests::no_active_stage_answers_registered_requests_as_inactive` | `test` |
| `viewport::app::project_stage::project_runtime_authority::tests::runtime_lease_rejects_a_synchronous_authoring_generation_change` | `viewport::app::project::runtime_authority::tests::runtime_lease_rejects_a_synchronous_authoring_generation_change` | `viewport::stage::project::runtime_authority::tests::runtime_lease_rejects_a_synchronous_authoring_generation_change` | `test` |

The exact map contains 38 unique C6 name migrations. That is 76 target-qualified applications per configuration across bin/lib, with the original six configuration scopes and target multiplicity retained. The migration map points directly from each pinned source identity to its C6 destination; no alias chains were introduced. The legacy and qualified baselines were not regenerated or edited.

## Ownership and verification scope

The test architecture policy now records the moved production owners and test targets. A read-only scan of `Cargo.toml` and `.github` found no explicit selectors tied to the old C6 source/test paths, so no Cargo target or CI selector changed. Consumer imports now point to `viewport::stage`; application composition calls `stage::project::install`, which installs LiveStage runtime systems before Project activation, preserving the former order.

The app test harness re-export now points to the moved `ProductionActivationWorld`. Activation and project modules remain internal to `viewport`; no crate-external API or serialized protocol changed. `UsdHubUI` remained read-only.

Compile was not run by the map-generation step. The assigned Python 3.14 `Validation.py --layer compile` result and final fingerprint will be recorded in the authoritative working plan and Local Agent handoff. Formal Test, live parity execution, commit, and push remain Local Agent gates.
