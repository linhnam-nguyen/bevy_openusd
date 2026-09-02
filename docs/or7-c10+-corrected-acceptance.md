# OR7-C10+ corrected acceptance packet

Status: implementation complete; Owner Review required. This packet records
the bounded OR7 correction batch after the rejected C10 integration. It does
not freeze OR7 and it does not start OR8.

## Reviewed bases and pushed checkpoints

- Backend branch: `develop/project-peerView`.
- Frontend branch: `projects-peerView`.
- Accepted backend C1+ base: `4d655c67ce3b3ded3796c254593ca5484d30ab5b`.
- Frontend C5+ correction: `267fe24`.
- Backend C8+: `525c558`; frontend C8+: `238a7fb`.
- Backend C9+: `88761ed`; frontend C9+: `537f98b`.
- The C10+ packet is the only remaining content for the next commits. Each
  repository is pushed immediately after its C10+ commit.

The reviewed C0-C7 history remains additive and unchanged. The root
`StageChangeBatch` / sparse-reconciliation architecture remains the receiving
architecture; the obsolete `PendingStageChanges` resync API was not restored.

## Corrected implementation record

### C5+ frontend hierarchy reveal

`HierarchyIndex` now builds and owns an anchor-to-index lookup together with
its existing parent/depth/subtree data. Search-clear selection reveal reuses
that index and walks parent links, so the interaction path is O(depth) after
the normal O(H) hierarchy-index rebuild. It no longer scans every read-model
node or allocates a whole-tree lookup map for each reveal. Focused reveal and
hierarchy tests passed: 10/10 and 28/28.

### C8+ complexity audit

`docs/or7-c8+-performance-complexity-audit.md` records the required per-path
input size, rejected/pre-correction shape, final complexity, allocations and
repeated work, invalidation, ownership, worker/thread behavior, and bounded
data behavior for backend root reconciliation, semantic sync, dense indexes,
search/BIM, lifecycle, native-instance dependencies, selection, and the
frontend hierarchy path. The audit distinguishes one-time index allocations
from interaction-time complexity and makes no numerical performance claim.

### C9+ integrated A-H matrix

`docs/or7-c9+-integrated-regression-matrix.md` records actual backend and
frontend A-H commands and results. The 40,000-node dense scene-index case
executed and passed. The remaining integrated backend semantic-consistency
runtime assertion is retained as a product/runtime blocker rather than being
reclassified as an environment issue.

## Final backend gates

| Gate | Result |
| --- | --- |
| `git diff --check` | Passed at the start of the final hardening run. |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check --workspace` | Passed; existing warnings only. |
| `cargo test --workspace` | Failed: 248 passed, 2 failed. The failures are `project::service::m19_tests::phase2_freeze_matrix_covers_create_import_composition_and_recovery` (`DirectoryNotEmpty` at `src/project/service/m19_tests.rs:103`) and `project::service::stage_mutation::tests::deleting_an_inactive_scene_is_consumed_without_mutating_the_active_stage` (assertion at `src/project/service/stage_mutation_tests.rs:226`). |
| `cargo check --workspace --no-default-features` | Passed; existing warnings only. |
| `cargo test --workspace --no-default-features` | Failed: 249 passed, 1 failed. The failure is the inactive-scene deletion assertion above. |
| `make check-source-size` | Passed: 772 files scanned, 82 warnings in the 351-400 review band, 0 failures over 400. |
| `make harden` | Failed at its `cargo test --workspace --no-default-features` stage with the inactive-scene deletion assertion. Because that stage failed, strict all-feature Clippy, all-feature tests, and the performance checker were not reached by `make harden`. |
| C9+ focused backend matrix | Passed focused dense index, native-instance, portability, authoring, migration, project activation, source-closure, animation, changed-info, and sparse-reconcile groups. The integrated `m8_semantic_consistency` test still fails at `src/viewport/api/bridge/tests/m8_semantic_consistency_test.rs:88` with `initial_projection.iter().any(|name| name == "1")`. |

These failures are recorded exactly and are outside this bounded documentation
checkpoint. No unrequested runtime behavior repair is included.

## Ownership and evidence boundary

The root receiving architecture, immutable snapshot handoffs, dense indexes,
bounded workers, activation generations, Project intent/acknowledgement
ownership, physical `Member_*` anchors, and independent BIM classification
recipe authority remain intact. The frontend does not own filesystem, Git, or
OpenUSD storage truth, and semantic sync consumes the typed root
`StageChangeBatch`.

This is local CPU-side compile, test, source-layout, and source-audit evidence.
It is not independent CI, browser, GPU, native Tauri, WebRTC, Revit,
Omniverse, or production evidence. The final C10+ commits preserve the
pre-existing frontend `Cargo.lock` and `src-tauri/Cargo.lock` edits as
unstaged, outside-scope user work.

OR7 IMPLEMENTATION COMPLETE
STATUS = OWNER REVIEW REQUIRED
STOP
