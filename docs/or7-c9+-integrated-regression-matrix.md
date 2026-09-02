# OR7-C9+ integrated Projects-root regression matrix

Status: executed and recorded. This matrix is intentionally a real test
matrix, not a source-coverage assertion. It covers the corrected integrated
backend and records the one remaining runtime failure without reclassifying it
as an environment limitation.

## Heads under test

- Backend branch: `develop/project-peerView`
- Backend head before this documentation checkpoint: `525c558`
- Frontend branch: `projects-peerView`
- Frontend head before this documentation checkpoint: `238a7fb`
- C1+ accepted backend base: `4d655c67ce3b3ded3796c254593ca5484d30ab5b`
- C5+ frontend correction: `267fe24`
- OR7 remains open; this matrix does not freeze OR7.

## A-H execution matrix

The rows are separate contracts. Counts are the direct result of the command
shown and are not added across rows because filters overlap.

| Row | Contract under test | Actual command(s) | Result |
| --- | --- | --- | --- |
| A | Direct Scene renderer opens with root hierarchy, selection/visibility paths, and current provider wiring | `cargo test --quiet --bin usdview project_and_direct_scene_activation -- --nocapture`; `cargo test --quiet --bin usdview scene_index -- --nocapture` | 1/1 passed; 10/10 passed. The dense 40,000-node case executed and passed. |
| B | Project-root presentation, root authority, and direct composition | `cargo test --quiet --bin usdview migrated_legacy -- --nocapture`; `cargo test --quiet --bin usdview authoring -- --nocapture`; frontend `cargo test -p usd_hub_desktop projects --quiet` | 2/2 passed; 15/15 passed; 166/166 passed. |
| C | Deep ordering/depth, bounded pages, virtual-window inputs, and selection reveal | Backend `scene_index` command above; frontend `cargo test -p usd_hub_desktop hierarchy --quiet`, `cargo test -p usd_hub_desktop reveal --quiet`, and `cargo test -p usd_hub_desktop scale --quiet` | Backend 10/10; frontend 28/28, 10/10, and 5/5 passed. |
| D | Hierarchy search is isolated from BIM search and classification | Backend `cargo test --quiet --bin usdview bim -- --nocapture`; frontend `cargo test -p usd_hub_desktop bim --quiet` plus `hierarchy` above | Backend 47 passed, 1 ignored, 1 failed; frontend 88/88 and 28/28 passed. The single backend failure is the existing semantic-consistency runtime assertion recorded below. |
| E | A -> B -> A activation, acknowledgement, stale response rejection, selection/search/page isolation | Frontend `cargo test -p usd_hub_desktop activation --quiet`, `coordinator --quiet`, `scene_cache --quiet`, and `m17 --quiet` | 23/23, 11/11, 13/13, and 10/10 passed. |
| F | Refresh/presentation/dense-index/generation invalidation | Backend `scene_index` command above and `cargo test -p usd_bevy --quiet`; frontend `scene_cache` and `activation` commands above | Backend 10/10 and 132 passed / 1 ignored; frontend 13/13 and 23/23 passed. |
| G | OR6 storage/portability, direct composition, import closure, localized references, export/reopen | `cargo test --quiet --bin usdview portability -- --nocapture`; `authoring -- --nocapture`; `migrated_legacy -- --nocapture`; `source_closure -- --nocapture` | 3/3, 15/15, 2/2, and 15/15 passed. This is CPU-side filesystem/authoring evidence only. |
| H | Animation, Standard4/Extended16-compatible projection routes, native instances, selection/visibility, sparse reconciliation | `cargo test -p usd_bevy animation --quiet`; `native_instance --quiet`; `changed_info --quiet`; `reconcile --quiet`; backend `native_instance` and `bim` filters above | 3/3, 15 passed / 1 ignored, 4/4, and 12/12 passed; native-instance scene selection 1/1 passed. The BIM-filtered backend result includes the one failure below. |

## Failure retained verbatim as a product/runtime blocker

The integrated backend command
`cargo test --quiet --bin usdview m8_semantic_consistency -- --nocapture`
compiled and ran one test, which failed:

`viewport::api::bridge::tests::m8_semantic_consistency_test::live_edit_converges_into_bim_classification_search_and_diff`

Failure:

`src/viewport/api/bridge/tests/m8_semantic_consistency_test.rs:88:5`

`assertion failed: initial_projection.iter().any(|name| name == "1")`

This is a real runtime assertion in the integrated test. It is not attributed
to the prior C1+ compile regression, and this bounded batch does not add an
unrequested semantic behavior fix. It therefore prevents an all-green OR7
acceptance packet at this checkpoint.

## Boundary results

- The C1+ correction is live: normal and no-default workspace compilation had
  already passed at `4d655c67`; the dense scene-index command now compiles and
  executes.
- The root `StageChangeBatch` / sparse-reconciliation architecture remains the
  receiving architecture. No old `PendingStageChanges` resync API was
  restored.
- Frontend C5+ selection reveal now executes through the memoized
  `HierarchyIndex` parent links and its focused tests pass.
- No browser, GPU, native Tauri, WebRTC, Revit, Omniverse, production, or
  independent CI evidence is claimed by these local CPU-side runs.

Owner Review remains required. OR7 is open and OR8 is not started.
