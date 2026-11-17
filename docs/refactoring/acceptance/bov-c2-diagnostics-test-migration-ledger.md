# BOV-C2 — Diagnostics and Viewport Test Migration Ledger

Mission: M-BOV-20261117-PC-CDS16-01  
Plan revision: sha256:3f7378770cdbafa638ef443f16e5a0cd60dee69fbaf938a3e6c8b6023c15c56b  
Checkpoint / iteration: BOV-C2 / 1  
Pre-change bevy_openusd HEAD: 91ec81166d1ccbe57ac209a79a212b177e8a9434

This successor record covers BOV-C2 only. The BOV-C1 ledger and earlier acceptance records remain unchanged.

## Scope and runtime ownership

Implemented phase 2 of the Owner-selected BOV plan: cluster the diagnostics/performance implementation and relocate its corresponding external viewport unit-test files in the same checkpoint.

The benchmark scenario and runner implementation remains production runtime code under src. The supported launch parser in src/viewport/transport/mod.rs accepts benchmark, renderer-matrix, and scenario options; src/viewport/app/runner.rs resolves BenchmarkScenarioId and registers ScenarioDriverPlugin and BenchmarkRunnerPlugin. No benchmark execution was moved to test tooling.

## Production source path mapping

| Previous path | BOV-C2 path |
| --- | --- |
| src/viewport/diagnostics/performance/aggregate.rs | src/viewport/diagnostics/performance/metrics/aggregate.rs |
| src/viewport/diagnostics/performance/collector.rs | src/viewport/diagnostics/performance/metrics/collector.rs |
| src/viewport/diagnostics/performance/counters.rs | src/viewport/diagnostics/performance/metrics/counters.rs |
| src/viewport/diagnostics/performance/matrix.rs | src/viewport/diagnostics/performance/matrix/mod.rs |
| src/viewport/diagnostics/performance/matrix_probe.rs | src/viewport/diagnostics/performance/matrix/probe.rs |
| src/viewport/diagnostics/performance/matrix_report.rs | src/viewport/diagnostics/performance/matrix/report.rs |
| src/viewport/diagnostics/performance/runner.rs | src/viewport/diagnostics/performance/runner/mod.rs |
| src/viewport/diagnostics/performance/runner_finalize.rs | src/viewport/diagnostics/performance/runner/finalize.rs |
| src/viewport/diagnostics/performance/runner_plugin.rs | src/viewport/diagnostics/performance/runner/plugin.rs |
| src/viewport/diagnostics/performance/sample.rs | src/viewport/diagnostics/performance/metrics/sample.rs |
| src/viewport/diagnostics/performance/scenario.rs | src/viewport/diagnostics/performance/scenario/mod.rs |
| src/viewport/diagnostics/performance/scenario_definitions.rs | src/viewport/diagnostics/performance/scenario/definitions.rs |
| src/viewport/diagnostics/performance/scenario_driver.rs | src/viewport/diagnostics/performance/scenario/driver.rs |
| src/viewport/diagnostics/performance/scenario_driver_setup.rs | src/viewport/diagnostics/performance/scenario/setup.rs |

The performance facade declares moved modules through explicit paths and retains their existing parent module names. A new metrics/mod.rs reexports aggregate, collector, counters, and sample under performance::metrics while preserving the existing performance::aggregate, performance::collector, performance::counters, and performance::sample paths. Matrix, runner, scenario, and their existing root helper modules likewise retain their prior logical module names. Existing imports and restricted visibility therefore remain valid without changing runtime schedules or behavior.

## External unit-test path mapping

| Previous test path | BOV-C2 path | SHA-256 after move |
| --- | --- | --- |
| tests/unit/viewport/diagnostics/performance/aggregate_inline_tests.rs | tests/unit/viewport/diagnostics/performance/metrics/aggregate_tests.rs | 096D5E1B1726AC4BEC40443DB6F0A3F254382FCE5DBCD0A3B3283BE5727C9A88 |
| tests/unit/viewport/diagnostics/performance/collector_inline_tests.rs | tests/unit/viewport/diagnostics/performance/metrics/collector_tests.rs | E8F84840C23BD87AA03F5B1AABEFAB6BBE2A4D1C416EED385710BC1AF0DA5187 |
| tests/unit/viewport/diagnostics/performance/counters_inline_tests.rs | tests/unit/viewport/diagnostics/performance/metrics/counters_tests.rs | 1B917AF63068F6078AE045E12AC1E2F8CE42F83DAF2F55CD253D99FCF62EF735 |
| tests/unit/viewport/diagnostics/performance/decision_inline_tests.rs | tests/unit/viewport/diagnostics/performance/decision_tests.rs | E3D06C4208EC1062C9D8AAD48A388070E0B4E0ACB0CBB1B6565EDADED2F0A7D5 |
| tests/unit/viewport/diagnostics/performance/matrix_tests.rs | tests/unit/viewport/diagnostics/performance/matrix/tests.rs | 07B88BB1FAEB708A0AD3D39255BA03099383C31F4EB53D973B75321D773D4E64 |
| tests/unit/viewport/diagnostics/performance/runner_tests.rs | tests/unit/viewport/diagnostics/performance/runner/tests.rs | F337F827EC06B52C797F186435F83AB3C50FCB90C0FD7166E0D965A798B7F6C3 |
| tests/unit/viewport/diagnostics/performance/sample_inline_tests.rs | tests/unit/viewport/diagnostics/performance/metrics/sample_tests.rs | 9ECD30A0717778ADCC3172FF657797816B2AC4BF0E1A495C9E3956D78EF94700 |
| tests/unit/viewport/diagnostics/performance/scenario_inline_tests.rs | tests/unit/viewport/diagnostics/performance/scenario/tests.rs | BC20121B86F3FF102E335AA8CE43126ACEA43DED746B5EFEA924DC88FCC2E11F |
| tests/unit/viewport/diagnostics/performance/scenario_driver_tests.rs | tests/unit/viewport/diagnostics/performance/scenario/driver_tests.rs | 2E15E366808474D4D6129472DACEF4340C44A44752A66C07D463BBD23D01A4F0 |

Each moved external test file has the same Git blob identity as its previous tracked path. Test source bytes are unchanged. The corresponding source attachments retain mod_name tests and module_path ""; only their path attributes and the reviewed architecture-policy paths changed.

## Test identity correspondence

All 18 test identities in the existing BO-M9-C1 legacy and qualified enumeration baselines retain their full names and occur with the same source-defined test functions:

| Stable logical module | Retained test functions | Count |
| --- | --- | ---: |
| viewport::diagnostics::performance::aggregate::tests | aggregation_median_and_p95_calculation; empty_sample_behavior_returns_zeroes_without_panic | 2 |
| viewport::diagnostics::performance::collector::tests | cache_snapshot_empty_world; phase_metrics_conversion_and_fallback | 2 |
| viewport::diagnostics::performance::counters::tests | query_latency_percentiles_are_derived_from_completed_samples; reset_preserves_configuration_flags | 2 |
| viewport::diagnostics::performance::decision::tests | ground_grid_decision_tolerance_invariants; semantic_decision_truth_table_invariants | 2 |
| viewport::diagnostics::performance::matrix::tests | cadence_samples_are_the_required_authority_targets; matrix_contains_all_sixteen_renderer_configurations | 2 |
| viewport::diagnostics::performance::runner::tests | benchmark_run_state_transitions | 1 |
| viewport::diagnostics::performance::sample::tests | benchmark_identity_records_git_and_glacial_sha; render_configuration_defaults_and_round_trip | 2 |
| viewport::diagnostics::performance::scenario::tests | all_24_scenarios_are_categorized_and_code_stable; scenario_probe_serialization_round_trip; steady_state_expectation_detects_mismatch | 3 |
| viewport::diagnostics::performance::scenario_driver::tests | s2_driver_disables_grid_on_startup; s4_driver_toggles_grid_visibility | 2 |
| Total |  | 18 |

The legacy and target-qualified parity baselines were not regenerated or edited. The existing test names were checked against the source-defined module attachments and baseline entries; the current harness was not listed as part of this Implementer assignment. Formal Test and live parity remain separate Local Agent gates.

## Policy and scope disposition

- Updated scripts/test_architecture_policy.json owner and target paths for the moved source/test attachments, plus the recorded sample-module location and consumers.
- The configured architecture guard reports zero violations after the moves.
- A path scan of .github and Cargo.toml found no references to the removed physical test/source paths.
- The historical docs/refactoring/backend-test-census.md remains unchanged; its paths describe the earlier census. This successor ledger records the BOV-C2 paths.
- UsdHubUI remains read-only.
- No manifest target names, feature gates, benchmark schedules, test functions, or test module names changed.
- No formal Test or live enumeration was run by the Implementer.

The final Compile result and source fingerprint are recorded in the authoritative working plan and the iteration Compile receipt.
