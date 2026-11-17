# BO-M9-C3 Acceptance Manifest: Zero `src/` Tests & Test-Identity Preservation

- **Mission:** `M-BO-M1-C0-20261109-CDS-LNN-01`
- **Checkpoint:** `BO-M9-C3` (Iteration 2)
- **Backend Repository (`bevy_openusd`):** branch `develop/refactoring`, commit `7097ae866a522a96cc67cfa5d933930e368002cb`
- **Paired UI Repository (`UsdHubUI`):** branch `refactoring`, commit `fa2faeef54e107cefbb0287ecf8830a0d45eba6f` (clean, read-only)
- **Pre-C0 assignment-start/API audit baseline:** `294d28e0134e4d18def81759820e88ad2a3c58eb`.
- **C0 closure / original test census baseline:** `1cbfd736279ae761b3ad4ae31e5708a84f43c0a5` (`docs/refactoring/backend-test-census.md`, Git blob `8e8b7d14acc2d7cdcf1cad6c6ff10c1ec66cb079`). The five C0 correction tests are already included.
- **C1 qualified enumeration baseline:** `59ccddbea7be85a34a8e8f1d4d131c91c7ade858`; the C3 live Windows receipts compare against this commit, not C0.

---

## 1. Executive Verdict on `src/` Purity

**Zero test implementations, test files, test directories, test attributes, or unreviewed test-only seams remain under any `src/` tree in `bevy_openusd`.**

| Verification Gate | Command / Mechanism | Result | Key Metrics |
| --- | --- | --- | --- |
| Canonical architecture guard | `python -B scripts/check_test_architecture.py --root . --json` | **PASS** (`exit 0`) | `violation_count: 0`, `cargo_metadata_used: true`, `workspace_members: 15`, `src_roots: 12`, `reviewed_path_only_attachments_count: 267`, `test_module_count: 338`, `tests_root_count: 10`, `exceptional_production_fixtures_count: 8`, `generated_sources_count: 1` |
| Rust integration wrapper | `cargo test --locked --test test_architecture` | **PASS** (`exit 0`) | `2 passed; 0 failed; 0 ignored` (`5.12s`) |
| Guard / parity / CI Python self-tests | `python -B -m unittest scripts/test_check_test_architecture.py scripts/test_test_enumeration_parity.py scripts/test_ci_workflow.py -v` | **PASS** (`exit 0`) | `Ran 50 tests in 2.587s`, `OK` |
| Independent filesystem + attribute scan | Direct walk of all 12 `src/` trees (`zero-src-tests-scan.json`) | **PASS** (`0` findings) | `585` files scanned (`583` `.rs`, `2` `.wgsl`); `0` `#[test]`/`#[tokio::test]`/`#[rstest]`/`#[test_case]`; `0` `src/**/tests/`; `0` `*_tests.rs`/`*_test.rs`/`test_support.rs`/`test_helpers.rs`/`test_fixtures.rs`; `0` non-path `#[cfg(test)]` |
| Support crate isolation | `cargo metadata --no-deps --format-version 1` + guard | **PASS** | `3` support crates (`project_support`, `usd_support`, `viewport_support`): all `publish = false`, zero `src/` directories, strictly `[dev-dependencies]` only |

---

## 2. Per-Crate `src/` Inventory (`7097ae866a522a96cc67cfa5d933930e368002cb`)

Comparison against `BO-M1-C0` (`docs/refactoring/backend-test-census.md` §§"Source-tree coverage" & "Source-tree test implementation and relocation map", `1cbfd73`):

| Package | `src/` Root | Files at `BO-M1-C0` (`1cbfd73`) | Files at `BO-M9-C3` (`7097ae86`) | `.rs` Files Now | External `#[cfg(test)] #[path]` Attachments | Non-Path `#[cfg(test)]` / `#[test]` in `src/` |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `usdview` | `src` | 558 | 364 | 364 | 223 | **0** |
| `usd_bevy` | `crates/usd_bevy/src` | 125 (incl. 2 `.wgsl`) | 94 (incl. 2 `.wgsl`) | 92 | 22 | **0** |
| `viewport_protocol` | `crates/viewport_protocol/src` | 36 | 35 | 35 | 1 | **0** |
| `viewport_streaming` | `crates/viewport_streaming/src` | 45 | 32 | 32 | 7 | **0** |
| `usd_project` | `crates/usd_project/src` | 12 | 10 | 10 | 5 | **0** |
| `project_protocol` | `crates/project_protocol/src` | 11 | 11 | 11 | 0 | **0** |
| `usd_semantic` | `crates/usd_semantic/src` | 10 | 9 | 9 | 4 | **0** |
| `usd_model` | `crates/usd_model/src` | 9 | 9 | 9 | 5 | **0** |
| `usd_diff` | `crates/usd_diff/src` | 8 | 8 | 8 | 0 | **0** |
| `usd_git` | `crates/usd_git/src` | 8 | 8 | 8 | 0 | **0** |
| `usd_rapier` | `crates/usd_rapier/src` | 4 | 4 | 4 | 0 | **0** |
| `usd_macro` | `crates/usd_macro/src` | 1 | 1 | 1 | 0 | **0** |
| **Total** | **12 `src/` trees** | **827** (`825` `.rs` + `2` `.wgsl`) | **585** (`583` `.rs` + `2` `.wgsl`) | **583** | **267** | **0** |

- Net reduction under `src/`: `827 -> 585` (`-242` net files: `243` test/support files relocated/removed out of `src/`, `+1` private production collaborator module `src/viewport/scene/projection_work.rs`).
- All `8` nested `src/**/tests/` directories documented in `BO-M1-C0` (`src/project/semantic_store/sync/tests`, `src/project/semantic_store/turso/tests`, `src/viewport/api/bridge/tests`, `src/viewport/semantic/tests`, `src/viewport/semantic/store/tests`, `crates/usd_bevy/src/live/tests`, `crates/viewport_streaming/src/application/tests`, `crates/viewport_streaming/src/data_channel/tests`) are **removed**.
- Shared test-support crates added in `BO-M6`: `tests/support/project` (`project_support`), `tests/support/usd` (`usd_support`), `tests/support/viewport` (`viewport_support`) — none contain a `src/` directory (`lib.rs` at crate root, `publish = false`).

---

## 3. Test-Identity Reconciliation (`BO-M1-C0` -> `BO-M9-C3`)

### 3.1 Static Source Attribute Reconciliation (`#[test]` / `#[tokio::test]`)

| Category | `BO-M1-C0` (`1cbfd73`) | `BO-M9-C3` (`7097ae86`) | Delta / Explanation |
| --- | ---: | ---: | --- |
| Under any `src/` tree | 1,086 | **0** | Relocated to `tests/` and `crates/*/tests/` (`BO-M2`–`BO-M5`) |
| Active external `tests/` & `crates/*/tests/` | 105 | 1,193 | `+1,086` relocated from `src/` + `+2` added in `tests/test_architecture.rs` (`BO-M9-C1`) |
| Test-support crates (`tests/support/*`) | 0 | 11 | `+3` `project_support`, `+6` `usd_support`, `+2` `viewport_support` (`+9` BO-M6, `+2` BO-M7) |
| **Active static test identities** | **1,191** | **1,204** | **`+13`** (`+11` support self-tests + `+2` architecture guard integration tests) |
| Dormant unregistered root `tests/*.rs` (`37` files, `autotests = false`) | 45 | 45 | `0` added, `0` removed; `1` blob unchanged (`tests/project_usdz.rs`), `36` normalized strictly in `BO-M7` (`a45daf85`) |
| **Total static workspace `#[test]` attributes** | **1,236** | **1,249** | Count reconciliation; identity-level proof is the ledger in §3.1.1 |

Breakdown of the `1,204` active unique test identities across disjoint compilation buckets (`scripts/test_enumeration_baseline.json`):
- `common_tests`: **`1,181`** unique identities (`1,958` target-qualified occurrences across default targets)
- `windows_only_tests`: **`4`** unique identities (`8` occurrences: `usdview::lib` + `usdview::bin`)
- `unix_only_tests`: **`11`** unique identities (`22` occurrences: `usdview::lib` + `usdview::bin`)
- `solari_feature_tests`: **`8`** unique identities (`16` occurrences: `usdview::lib` + `usdview::bin`)
- Sum: `1,181 + 4 + 11 + 8 = 1,204` active unique test identities in the six-configuration union. The ledger below establishes the mapping; counts alone do not.

### 3.1.1 Exact original-to-current identity ledger (F-01)

The adjacent [`bo-m9-c3-test-identity-ledger.tsv`](bo-m9-c3-test-identity-ledger.tsv) is the explicit 1,249-row source-definition ledger (not a count-only assertion). Its original identities are `(C0 source file, function name)` from the exact C0 closure census, including the 45 dormant definitions; executable destinations are full Cargo-qualified identities `(manifest, target kind, target name, qualified test name)` with an explicit six-configuration membership list. Historical source paths are not assumed to be current files. The C0 target is derived from the census's original registration and source ownership (root source lib+bin, member source lib, external target registration), not fabricated from the current name.

- The 1,191 registered/gated original definitions have unique leaf names after removing census gate annotations. Exact leaf equality within the same owning package or one of the 22 explicit function renames taken from BO-M8 commit `2ac26f305f9a5201ed005b1d136f876c583e7975` maps each to one unique current name; no ambiguity, duplicate mapping, missing or extra unapproved active identity. Full destination names and target sets are from the immutable C1 schema-2 capture, verified byte-for-byte against the file at C3 HEAD (Git blob `3bbf4306ddb1dbe8ed988d6bab2d0595776ad913`, SHA-256 `5e9f340f52b74e78c43a6f5be6b8abe5ed26c830e03b7ec9d7b52a96dde633b9`). The schema-1 cross-check blob is `f0e323a1744a8e3f2cb758f68122a07ece4219be`.
- 61 source identities have target moves/renames: 21 project_protocol lib → contracts; 12 viewport_protocol lib → relocated_contracts; 18 viewport_streaming lib → relocated_contracts; 7 root m8_instancing_correctness → point_instancer_correctness; 1 each m10_persistent_soak → persistent_runtime_soak, m8_instancing_freeze → point_instancer_reconcile_profile, m8_instancing_profile → point_instancer_profile. Three also have a function rename. Other module-only name changes are represented by the full current qualified identity, not counted as leaf renames. The BO-M8 authoritative working plan separately records the approved full-identity bijection of 56 names/105 occurrences; the 22 leaf renames are a subset, not a replacement for that receipt.
- Exactly 13 additional current definitions are individually identified: 11 support tests (+9 BO-M6, +2 BO-M7) and 2 architecture wrapper tests (BO-M9-C1). No matching by an aggregate count or fuzzy name similarity. Each of the six configuration projections equals the corresponding exact C1 target-qualified capture as a multiset, including both root harnesses. The two Windows current live parity receipts establish C1→C3 equality; four Linux/macOS projections remain synthetic drafts, not native execution.
- 45 dormant source definitions across 37 unregistered root `tests/*.rs` files have explicit original path/name rows and no invented executable identity. Across `C0..C3`, `1` file (`tests/project_usdz.rs`) retains a byte-identical Git blob (`C0 == C3`), and the other `36` files changed strictly once in `BO-M7` commit `a45daf853c9330832e69e71d9a370958467548fc` (`refactor: normalize test fixtures and fixture helpers (BO-M7)`, replacing `"tests/stages"` / fixture path strings with `usd_support` fixture helpers across `46` removed / `46` added lines), with all `45` `#[test] fn` definitions still present at `C3`. `verify_bo_m9_c3_ledger.py` enforces both this exact `C0 == C3` or sole-`BO-M7`-commit invariant and the presence of all `45` `#[test]` definitions at `C3`; any other missing or altered dormant file fails verification. No execution claim is made for dormant sources.
- The BO-M1-C3 access-tier ledger's 1,237 records include the single `OPENUSD_STAGE_SEND_PROBE` compile-fail Rustdoc contract (row 342, authoritative working plan BO-M1-C3 I4/I5 and BO-M1-C4 inventory), in addition to 1,236 static `#[test]` definitions. The C0 and C3 exact source declarations retain `/// ```compile_fail`, `requires_send::<openusd::usd::Stage>();` and `OPENUSD_STAGE_SEND_PROBE` in `src/project/service/stage_activation.rs`. This is not a missing static test. The separate `usd_macro` Rustdoc example is the second C0 doctest listing row; neither doctest belongs in the static `#[test]` or schema-2 executable-test counts. Doctest execution remains separate from this ledger.

**Reproduction and handoff:** From `bevy_openusd` at exact HEAD `7097ae866a522a96cc67cfa5d933930e368002cb`, run `python -B docs/refactoring/acceptance/verify_bo_m9_c3_ledger.py --root .` to check the ledger against committed C0 census, exact current schema-1/schema-2 manifests, explicit BO-M8 rename list, source-owner/target rules, all 37 dormant files (`1` unchanged + `36` `BO-M7`-normalized, `45` definitions), and the visibility ledger. No network or Cargo execution is needed for this data check. Preserve a successful JSON receipt with the corrected packet; if any invariant fails, stop and investigate, do not regenerate expected identities from the current list. The supplied ledger is a concrete mapping; local Git verification, fresh matching Compile and formal Test remain to be executed by the appropriate agent. No final milestone acceptance is claimed.

### 3.2 Live Target-Qualified Enumeration Parity (`scripts/test_enumeration_qualified_baseline.json`)

Live execution on Windows (`x86_64-pc-windows-msvc`, `CARGO_BUILD_JOBS=1`) via `scripts/check_test_enumeration_parity.py`:

| Configuration | Live / Baseline Provenance | Targets (Non-Empty / Total) | Total Occurrences | Unique Names | Result |
| --- | --- | --- | ---: | ---: | --- |
| `windows:default` | `NATIVE_CAPTURED` (`native_host_verified: true`) + live re-run | `36 / 43` | **1,966** | **1,185** | **PASS** (`violation_count: 0`, `0` missing, `0` unexpected) |
| `windows:solari` | `NATIVE_CAPTURED` (`native_host_verified: true`) + live re-run | `36 / 43` | **1,982** | **1,193** | **PASS** (`violation_count: 0`, `0` missing, `0` unexpected) |
| `linux:default` | `DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST` (`native_host_verified: false`) | `36 / 43` | 1,980 | 1,192 | Offline parity checked in `BO-M9-C2` (awaiting native Linux capture) |
| `linux:solari` | `DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST` (`native_host_verified: false`) | `36 / 43` | 1,996 | 1,200 | Offline parity checked in `BO-M9-C2` (awaiting native Linux capture) |
| `macos:default` | `DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST` (`native_host_verified: false`) | `36 / 43` | 1,980 | 1,192 | Offline parity checked in `BO-M9-C2` (awaiting native macOS capture) |
| `macos:solari` | `DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST` (`native_host_verified: false`) | `36 / 43` | 1,996 | 1,200 | Offline parity checked in `BO-M9-C2` (awaiting native macOS capture) |

Note on occurrence multiplicity: `usdview` compiles its crate unit tests into both `lib::usdview` (`781` default / `789` solari on Windows) and `bin::usdview` (`781` default / `789` solari on Windows), which accounts for the exact `1,966 - 1,185 = 781` (`1,982 - 1,193 = 789`) difference.

### 3.3 Target-Qualified Per-Package Breakdown (`windows:default`, `1,966` Occurrences)

| Package / Manifest | Targets | Non-Empty Targets | Enumerated Tests (`windows:default`) |
| --- | --- | --- | ---: |
| `usdview` (`Cargo.toml`: `lib`, `bin::usdview`, `bin::live_editor`, `example::pack_usdz`, `11` `[[test]]` targets) | 15 | 13 | 1,584 (`781` lib + `781` bin + `22` integration) |
| `usd_bevy` (`crates/usd_bevy/Cargo.toml`) | 1 | 1 | 140 |
| `viewport_protocol` (`crates/viewport_protocol/Cargo.toml`: `lib` + `7` `tests/*.rs`) | 8 | 8 | 73 (`2` lib + `71` integration) |
| `viewport_streaming` (`crates/viewport_streaming/Cargo.toml`: `lib` + `relocated_contracts`) | 2 | 2 | 58 (`40` lib + `18` integration) |
| `usd_project` (`crates/usd_project/Cargo.toml`) | 1 | 1 | 23 |
| `project_protocol` (`crates/project_protocol/Cargo.toml`: `lib` + `contracts`) | 2 | 1 | 21 (`0` lib + `21` integration) |
| `usd_semantic` (`crates/usd_semantic/Cargo.toml`: `lib` + `identity`) | 2 | 2 | 21 (`18` lib + `3` integration) |
| `usd_git` (`crates/usd_git/Cargo.toml`: `lib` + `3` `tests/*.rs`) | 4 | 3 | 16 (`0` lib + `16` integration) |
| `usd_model` (`crates/usd_model/Cargo.toml`) | 1 | 1 | 12 |
| `usd_diff` (`crates/usd_diff/Cargo.toml`: `lib` + `diff_matrix`) | 2 | 1 | 7 (`0` lib + `7` integration) |
| `usd_macro` / `usd_rapier` | 2 | 0 | 0 |
| `usd_support` (`tests/support/usd/Cargo.toml`) | 1 | 1 | 6 |
| `project_support` (`tests/support/project/Cargo.toml`) | 1 | 1 | 3 |
| `viewport_support` (`tests/support/viewport/Cargo.toml`) | 1 | 1 | 2 |
| **Total** | **43** | **36** | **1,966** (`1,185` unique) |

---

## 4. Paired `UsdHubUI` Test Inventory (`fa2faeef54e107cefbb0287ecf8830a0d45eba6f`)

| Suite / Package | Enumerated / Executed | Passed | Failed | Ignored / Skipped | Notes |
| --- | ---: | ---: | ---: | ---: | --- |
| `usd_hub_desktop` (`apps/desktop`, Rust workspace) | 555 | 554 | 0 | 1 | `profile_i1_7_1_frontend_large_multi_selection_baseline` (`#[ignore]`) |
| `usd_hub_desktop_web` (`apps/desktop` bin) | 0 | 0 | 0 | 0 | Thin WASM entrypoint |
| `viewport_client` (`crates/viewport_client`, Rust workspace) | 11 | 11 | 0 | 0 | Includes `protocol_tests` & `editor_tests` |
| `usd_hub_desktop_host` (`src-tauri`, native Tauri host) | 2 | 2 | 0 | 0 | `project_host` & `project_read` registration checks |
| UI script self-tests (`fluent:tokens:test`, `fluent:icons:test`, `rust:test-source-layout:test`, `frontend:build:test`) | 32 | 31 | 0 | 1 | `test_check_rust_test_source_layout` has 1 conditional skip |
| Playwright `@a11y` (`tests/ui/accessibility.spec.js`) | 23 | 23 | 0 | 0 | Axe + keyboard focus + forced-colors + 200% reflow |
| Playwright full `test:ui:e2e` (`accessibility.spec.js` + `fluent.spec.js` + `navigation.spec.js`) | 61 | 26 | 35 | 0 | See `bo-m9-c3-paired-build-and-smoke-report.md` §3.3 |
