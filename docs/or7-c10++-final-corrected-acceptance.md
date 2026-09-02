# OR7-C10++ final corrected acceptance packet

Status: implementation complete; Owner Review required. This packet records
the final bounded OR7 correction batch. It does not freeze OR7, start OR8, or
rewrite any accepted history.

## Pushed checkpoints

- Backend branch: `develop/project-peerView`.
- Backend C6+ correction: `5429446` — `Owner review 7-C6+: resolve BIM classification hierarchy contract regression`.
- Backend C9++ matrix: `8102b13` — `Owner review 7-C9++: validate corrected semantic consistency matrix`.
- Backend C10++: the commit carrying this packet, using the exact message
  `Owner review 7-C10++: record final corrected OR7 acceptance`.
- Frontend branch: `projects-peerView`.
- Frontend is unchanged at `7ad225b01c752632d1b0cd259d9fed60a18b041c`.
- Existing frontend `Cargo.lock` and `src-tauri/Cargo.lock` edits remain
  outside this backend correction batch.

All three backend correction checkpoints are additive and are pushed to
`origin/develop/project-peerView` immediately after their commits. Reviewed
C0-C7 history, the root `StageChangeBatch`/sparse-reconciliation receiving
architecture, and accepted C5+/C8+/C9+ work remain intact.

## Corrected C6+ implementation

The original `m8_semantic_consistency` assertion coupled BIM classification
to contextual hierarchy projection. The corrected test uses the accepted
authority split: `SetBimClassificationRecipe` updates the independent BIM
recipe state while `ActiveHierarchyProvider` and
`CurrentHierarchyProjection` remain on the contextual Prim source.

The correction also fixes the production command route: the typed
`SetBimClassificationRecipe` command is now admitted to the scene-query
inbox, where the existing BIM classification dispatcher consumes it. The
obsolete `PendingStageChanges` resync API was not restored.

The corrected live-edit integration proves initial BIM extraction, recipe
storage, contextual hierarchy preservation, the `/World/A.Width` 1 -> 10
`StageChangeBatch`, `LiveStage`, canonical `SemanticSnapshot`, semantic diff,
BIM read/classification update, `RequestBimProperties`, BIM search, and
generation-2 rejection of a stale generation-1 BIM result. The focused
semantic consistency result is 2/2 passed.

## Final gates

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check --workspace` | Passed; existing warnings only. |
| `cargo test --workspace` | Failed after execution: 248 passed, 2 failed. `project::service::m19_tests::phase2_freeze_matrix_covers_create_import_composition_and_recovery` panicked at `src/project/service/m19_tests.rs:103:60` with `DirectoryNotEmpty`; `project::service::stage_mutation::tests::deleting_an_inactive_scene_is_consumed_without_mutating_the_active_stage` failed at `src/project/service/stage_mutation_tests.rs:226` because the exported stage still contained `SceneRoot`. |
| `cargo check --workspace --no-default-features` | Passed; existing warnings only. |
| `cargo test --workspace --no-default-features` | Failed after execution: 248 passed, 2 failed, with the same two exact test signatures above. |
| `make check-source-size` | Passed: 772 files scanned, 82 warnings in the 351-400 review band, 0 failures over 400. |
| `git diff --check` | Passed before the C10++ commit. |
| `make harden` | Failed at its `cargo test --workspace --no-default-features` stage on the same two failures. Its preceding diff-check, formatting, source-size, and no-default compilation stages passed; strict all-feature Clippy, all-feature tests, and performance regression checks were not reached. |

The M19 and inactive-scene failures are test failures, not environment
claims. Their signatures match the known final-integration failures recorded
before this correction; they are outside the bounded C6+ authority repair.

## Corrected OR7 matrix and focused evidence

The complete corrected A-H evidence is recorded in
[`or7-c9++-corrected-semantic-consistency-matrix.md`](or7-c9++-corrected-semantic-consistency-matrix.md).
The relevant current backend results are:

- semantic filter suite: 88/88 passed;
- BIM suite: 49 passed, 1 ignored, 0 failed;
- scene-index suite: 10/10 passed, including the 40,000-node case;
- `usd_bevy` package suite: 132 passed, 1 ignored, 0 failed;
- corrected `m8_semantic_consistency`: 2/2 passed;
- hierarchy suite: 12 passed and 1 inherited deterministic C0 assertion
  failure at `hierarchy_search_test.rs:234` for the old `Exterior Wall`
  non-match expectation. This was not changed in the C6+ batch.

The frontend has no C6+, C9++, or C10++ source/documentation checkpoint in
this batch; its reviewed head remains the unchanged pushed SHA above.

## Ownership and evidence boundary

LiveStage remains authoritative uncommitted USD working state; Git remains
authoritative committed Project history; canonical Project/OpenUSD files
remain portable Project state; semantic/Turso and BIM indexes remain derived
query state; Bevy ECS remains disposable rendering projection. The frontend
does not own filesystem, Git, or OpenUSD truth. Mutations continue through
`StageSink` -> `StageChangeBatch` -> renderer reconciliation, semantic sync,
and recovery.

These are local CPU-side compile, test, source-size, and source-audit results.
They do not prove browser, GPU, native Tauri, WebRTC, Revit, Omniverse,
production, or independent CI behavior.

OR7 IMPLEMENTATION COMPLETE
STATUS = OWNER REVIEW REQUIRED
STOP
