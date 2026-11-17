# BO-M9-C3 Acceptance Manifest: API-Visibility Diff Audit (`BO-M1-C0` -> `BO-M9-C3`)

- **Checkpoint:** `BO-M9-C3` (Iteration 2)
- **Diff Range:** `294d28e0134e4d18def81759820e88ad2a3c58eb..7097ae866a522a96cc67cfa5d933930e368002cb` (`-- src crates`)
- **Backend HEAD:** `7097ae866a522a96cc67cfa5d933930e368002cb` (`develop/refactoring`)
- **Paired UI HEAD:** `fa2faeef54e107cefbb0287ecf8830a0d45eba6f` (`refactoring`)
- **Raw Audit Data:** `AgentsComm/communication/agents/implementer/BO-M9-C3-I1/visibility-audit.json` & `visibility-classified.json`

---

## 1. Executive Verdict

**PASS — raw visibility-hit accounting is complete; the single unconditional public hit is an unchanged re-export. No semantic public addition is demonstrated by this classified audit.** Wire/behavioral compatibility is assessed separately by paired compile, contract and runtime evidence; a visibility-line audit is not a proof of complete wire equivalence.

Across all `486` changed entries under `src/` and `crates/*/src/` (`243` relocated/deleted test/support files + `243` modified or added production files):
1. **Unconditional `pub` additions across all crate roots and modules:** **`0`** semantic additions (the single `pub use` diff hit in `crates/usd_semantic/src/lib.rs:26` is a `rustfmt` alphabetical line reorder of an existing re-export).
2. **UI-shared crates (`viewport_protocol`, `project_protocol`, `usd_project`):** **`0`** added or removed `pub` / `pub(crate)` / `pub(super)` items; all `13` modified `src/` files only updated `#[cfg(test)]` external `#[path = "..."]` attachments or removed inline test bodies.
3. **Unconditional restricted visibility (`41` raw added line hits across `22` files):** `22` `pub(crate)` and `19` `pub(super)`. The detailed catalog contains `15` restricted reorder/reflow/retained-visibility hits across 12 files, `4` Windows migration-guard hits, and `22` private collaborator/replacement hits. Including the one public reorder, there are `42` unconditional raw additions across `23` files. These are changed declaration lines, not net semantic additions.
4. **`#[cfg(test)]`-gated visibility items added (`18`):** strictly `pub(super)` / `pub(in crate::viewport)` / `pub(crate)` external `#[path = "..."]` module declarations (`mod tests;`, `mod test_support;`, `mod test_harness;`, `mod production_tests;`), excluded from non-test builds and enforced by `scripts/check_test_architecture.py`.
5. **Raw removed visibility hits (`98`):** this includes matching public/restricted reorder and reflow lines, production replacements and test-seam removals. No claim that 98 test-only declarations were deleted. Every hit is enumerated in the checked-in visibility ledger.

---

## 2. UI-Shared Crates (`viewport_protocol`, `project_protocol`, `usd_project`)

| Crate | Modified `src/` Files | Added Visibility Line Hits | Removed Visibility Line Hits | Exact Change Nature |
| --- | --- | ---: | ---: | --- |
| `project_protocol` | `src/activation.rs`, `src/lib.rs`, `src/placement.rs` | 0 | 0 | Removed inline `#[cfg(test)] mod tests { ... }` blocks (relocated to `tests/contracts.rs`) |
| `usd_project` | `src/composition.rs`, `src/lib.rs`, `src/manifest.rs`, `src/read_model.rs` | 0 | 0 | Replaced inline `#[cfg(test)] mod tests` / `src/validation.rs` / `src/manifest_tests.rs` with `#[path = "../tests/..."]` |
| `viewport_protocol` | `src/authorization.rs`, `src/delivery.rs`, `src/runtime_client/mod.rs`, `src/semantic_sync.rs`, `src/stream.rs`, `src/viewport/bim/read_model.rs` | 0 | 0 | Removed inline `#[cfg(test)] mod tests` / updated `#[path = "../tests/unit/stream_tests.rs"]` (relocated to `tests/contracts/` & `tests/unit/`) |

---

## 3. Complete Catalog of Unconditional Visibility Diffs Across All Backend `src/` Trees

### 3.1 Unconditional `pub` Diffs (`1` Formatting Reorder, `0` Semantic Additions)

| File & Line | Item | Classification & Rationale |
| --- | --- | --- |
| `crates/usd_semantic/src/lib.rs:26` | `pub use transform::extract_transform;` | **Identical declaration-line reorder:** moved from line 21 to line 26 alphabetically after `pub use nvidia::{...}`. Zero API change. |

### 3.2 `rustfmt` Multi-Line -> Single-Line Reflows (`15` Restricted Line Hits Across 12 Files)

| File & Line | Item | Rationale |
| --- | --- | --- |
| `src/project.rs:5,14` | `pub(crate) mod cache_compatibility;`, `pub(crate) mod cache_warm_runtime;` | Alphabetical module declaration reflow (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/project_api.rs:23,37` | `pub(crate) mod cache_contract;`, `pub(crate) mod cache_warm_runtime;` | Alphabetical module declaration reflow (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/project/cache.rs:32` | `pub(crate) use scene_store::SceneCacheStore;` | Alphabetical re-export reflow (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/project/cache_scene_payload.rs:73` | `pub(crate) fn prepare_animation_payload(...)` | Single-line signature wrap (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/project/service/rename.rs:46` | `pub(super) fn rename(...)` | Line shift/wrap after removing `#[cfg(test)]` hook helpers (`BO-M9-C1 I2`); unchanged `pub(super)` visibility. |
| `src/project/service/stage_activation.rs:36` | `pub(crate) const OPENUSD_STAGE_SEND_PROBE: ...` | Single-line wrap (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/viewport/residency/catalog.rs:87,132` | `pub(crate) fn from_presentation(...)`, `pub(crate) fn descriptors_for(...)` | Single-line wrap (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/viewport/residency/mod.rs:45` | `pub(crate) use catalog::{...};` | Alphabetical re-export reflow (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/viewport/residency/repair.rs:25` | `pub(crate) use repair_handoff::{...};` | Alphabetical re-export reflow (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/viewport/residency/repair_phase.rs:21` | `pub(crate) fn lookup_parts(&self) -> ...` | Single-line wrap (`rustfmt`); unchanged `pub(crate)` visibility. |
| `src/viewport/scene/solari_projection.rs:59` | `pub(super) fn sync_solari_usd_projection(...)` | Reflowed signature delegating to `projection_work` (`BO-M3-C3 I2`); unchanged `pub(super)` visibility. |
| `src/viewport/session/mod.rs:8` | `pub(crate) use cache_presentation_gate::CachePresentationGate;` | Alphabetical re-export reflow (`rustfmt`); unchanged `pub(crate)` visibility. |

### 3.3 Authorized `BO-M1-C0 I4` Windows Fail-Closed Migration Guard (`4` Internal Items)

| File & Line | Item | Scope & Risk Assessment |
| --- | --- | --- |
| `src/project/storage/migration.rs:26` | `pub(super) const WINDOWS_MIGRATION_UNSUPPORTED: &str` | Crate-internal (`pub(super)`) error constant for `BO-M1-C0 I4` Windows durability guard; zero external exposure. |
| `src/project/storage/migration.rs:151,156` | `pub(crate) fn ensure_legacy_migration_supported() -> Result<()>` | Paired `#[cfg(windows)]` / `#[cfg(not(windows))]` crate-internal gate (`BO-M1-C0 I4`); zero `pub` exposure. |
| `src/project/storage.rs:24` | `pub(crate) use migration::{..., ensure_legacy_migration_supported, migrate_legacy_project_with_observer, ...};` | Re-exports crate-internal migration helpers to `src/project` callers; zero `pub` exposure. |

### 3.4 Authorized `BO-M9-C1 I2` Option 1 Minimal Private Collaborators (`22` Restricted Line Hits, Zero `pub`)

Added under `OWNER_DESIGN_DECISION: OPTION_1_MINIMAL_PRIVATE_COLLABORATORS` to eliminate all `17` `#[cfg(test)]` / `#[cfg(not(test))]` production seams without widening any crate's public (`pub`) API:

| File & Lines | Added Internal Items | Visibility Scope | Risk Assessment |
| --- | --- | --- | --- |
| `crates/usd_bevy/src/live/reconcile/mod.rs:21,77,81,89,101,395` | `trait ReconcileExecutor`, `struct WorldReconcileExecutor`, `FullWork`, `SubtreeWork`, `SparseWork`, `fn apply_change_batch_with_executor` | `pub(super)` (within `usd_bevy::live`) | Replaces `ReconcileStats` resource seam (`BO-M4-C5 I2`); crate-private, zero `usd_bevy` `pub` change. |
| `crates/usd_bevy/src/live/reconcile/full.rs:19,69` | `fn reconcile_full`, `fn execute_full_work` | `pub(super)` | Internal executor split within `live::reconcile`. |
| `crates/usd_bevy/src/live/reconcile/subtree.rs:157` | `fn execute_subtree_work` | `pub(super)` | Internal executor split within `live::reconcile`. |
| `crates/usd_bevy/src/live/system.rs:57` | `fn reproject_from_batch_system<E: ReconcileExecutor + Default>` | `pub(super)` | Parameterizes existing `pub(super)` system over `ReconcileExecutor`. |
| `src/project/catalog/manifest_store.rs:34` | `fn read_validated_with_observer<O: MigrationPublishObserver>` | `pub(crate)` | Allows migration recovery tests to inject observer without `#[cfg(test)]` hook. |
| `src/project/service/stage_mutation_outbox.rs:19,26` | `fn submit_batch_locked`, `fn submit_batch_locked_with<F>` | `pub(super)` | Replaces `test_fail_before_index` `#[cfg(test)]` field/stub with closure seam. |
| `src/project/storage/migration.rs:70,160` | `fn migrate_legacy_project_with_observer<O>`, `pub(crate) use publish::{...}` | `pub(crate)` | Replaces `failure_injection` `#[cfg(test)]` module with static `MigrationPublishObserver` trait. |
| `src/project/storage/migration_publish.rs:16,23,27,36` | `enum MigrationPublishCheckpoint`, `trait MigrationPublishObserver`, `struct NoopMigrationPublishObserver`, `fn publish_plan_with_observer<O>` | `pub(crate)` / `pub(super)` | Replaces 4 `#[cfg(not(test))]` no-op stubs with zero-cost ZST `NoopMigrationPublishObserver`. |
| `src/viewport/scene/projection_work.rs:7,18,25` | `enum ProjectionBatch`, `trait ProjectionWork`, `struct DirectProjectionWork` | `pub(super)` | Replaces 3 `#[cfg(test)]` projection stats resources (`BO-M3-C3 I2`). |

---

## 4. `#[cfg(test)]`-Gated Path Attachments & Narrowed Internal Surface

- **`18` `#[cfg(test)]`-gated module visibility items added:** e.g. `#[cfg(test)] pub(super) mod test_support;` (`cache_warmer_queue.rs`, `runtime_authority_queue_storage.rs`, `stage_mutation.rs`, `project_activation.rs`, `project_activation_flow.rs`, `project_runtime_snapshot.rs`, `solari_projection.rs`, `visualization_render_mode.rs`, `visualization_shadows.rs`, `crates/usd_bevy/src/live/reconcile/mod.rs`), `#[cfg(test)] pub(super) mod tests;` (`bridge/mod.rs`), `#[cfg(test)] pub(super) mod production_tests;` (`project_activation.rs`), `#[cfg(test)] pub(in crate::viewport) mod tests;` (`animation/mod.rs`), `#[cfg(test)] pub(in crate::viewport) mod test_harness;` (`viewport/api/mod.rs`, `viewport/app/mod.rs`), `#[cfg(test)] pub(crate) mod test_harness;` (`viewport/mod.rs`), `#[cfg(test)] pub(crate) mod test_support;` (`semantic_store/mod.rs`), `#[cfg(test)] pub(crate) mod tests;` (`viewport_streaming/src/application/mod.rs`). None exist in non-test builds, and all `267` `#[cfg(test)] #[path]` attachments are verified by `scripts/check_test_architecture.py` to resolve outside all `src/` roots.
- **Examples of reported test-support/seam removals (not a count of semantic removals):** including `ReconcileStats` (`usd_bevy`), `ApplicationSession::new` (`viewport_streaming`), `ProjectCacheWarmQueue::wait_for`, `TursoSemanticStore::connection`, `ProjectRuntimeAuthorityQueue::{with_timeout, with_timeout_and_registry_path, registry_resolution_count, prepare_request_claim, cancel_request_for_test, cleanup_for_test}`, `ProjectStageMutationQueue::fail_before_batch_index`, `SceneAnchorIndex::{from_test_nodes, from_test_entity, from_test_entities, ...}`, `CachedResidencyWorker::{unavailable_for_test, with_completion_for_test}`, `SelectedRenderableProjection` test accessors, `SemanticSyncState::from_test_snapshot`, and `CachePresentationGate::rendered`.

## 5. Exact raw-hit ledger and bounded semantic interpretation

Source: frozen `visibility-classified.json` (SHA-256 `dc5e213893ec8c56f2bf3dd30029340d2842708e4e56c8f294128dc679b9047c`), 65 file records. The included `bo-m9-c3-visibility-classified.json` preserves the exact JSON data with LF line endings; restoring CRLF reproduces the frozen source SHA-256. [`bo-m9-c3-visibility-hit-ledger.tsv`](bo-m9-c3-visibility-hit-ledger.tsv) records every one of the 158 added/removed hits by file, side and ordinal; ordinal (not the possibly absent/repeated line number) is the record identity. Added-side `cfg_test` is copied from the source; the removed side has no such metadata and is intentionally blank.

| Classification | Added hits | Removed hits |
| --- | ---: | ---: |
| `DOCUMENTED_PRIVATE_REPLACEMENT` | 0 | 6 |
| `OTHER_REMOVED_RESTRICTED_LINE` | 0 | 76 |
| `PRIVATE_COLLABORATOR_OR_REPLACEMENT` | 22 | 0 |
| `PUBLIC_IDENTICAL_REEXPORT` | 1 | 1 |
| `RESTRICTED_REORDER_REFLOW_OR_SAME_VISIBILITY` | 15 | 15 |
| `TEST_GATED_PATH_ATTACHMENT` | 18 | 0 |
| `WINDOWS_MIGRATION_GUARD` | 4 | 0 |
| **Total raw hits** | **60** | **98** |

The ledger distinguishes the public exact-line pair, reported restricted reflows/retained-visibility hits, known private replacements, test-gated attachments, and other raw restricted removals. `OTHER_REMOVED_RESTRICTED_LINE` is an explicit raw-data disposition, not proof that an item is test-only or semantically deleted. The old snippets lack complete signatures, cfg and use context; do not infer a total of true semantic test-only deletions from them. In particular, the removed public re-export and restricted formatting pairs are not semantic deletions. No unclassified raw hit remains. A stronger semantic claim requires full declarations, module ownership, cfg and use evidence from the exact Git range. The exact `pub` pair and zero shared-crate visibility-line hits are supported; complete wire equivalence is not asserted from this audit.

**Audit conclusion:** accounting PASS for all 60 additions and 98 removals; 42 unconditional additions (1 public reorder + 41 restricted) and 18 test-gated attachments. No semantic new public declaration is demonstrated by this classified scan. The paired Compile/contract and actual smoke statuses remain separate. No new production changes are proposed by this audit.
