# OR7-C9++ corrected semantic-consistency matrix

Status: executed and recorded. This is the final bounded OR7 correction
matrix after the C6+ BIM classification authority correction. It preserves
the accepted C0-C9+ history, does not rewrite earlier evidence, and does not
freeze OR7.

## Heads under test

- Backend branch: `develop/project-peerView`.
- Backend C6+ correction: `5429446` — `Owner review 7-C6+: resolve BIM classification hierarchy contract regression`.
- Backend head at this documentation checkpoint: `5429446`.
- Frontend branch: `projects-peerView`.
- Frontend remains unchanged at `7ad225b01c752632d1b0cd259d9fed60a18b041c`.
- Accepted backend C1+ base: `4d655c67ce3b3ded3796c254593ca5484d30ab5b`.
- OR7 remains open; this matrix does not freeze OR7.

## C6+ correction contract

The original integrated assertion treated a BIM classification recipe as if it
replaced the contextual hierarchy provider. The accepted C6 authority split
requires `SetBimClassificationRecipe` to update BIM presentation state while
leaving `ActiveHierarchyProvider` and `CurrentHierarchyProjection` on the
contextual source. The test now exercises that split and the typed command is
routed through the scene-query inbox. The root `StageChangeBatch` and sparse
reconciliation architecture remains the receiving architecture; no obsolete
`PendingStageChanges` resync API was restored.

The corrected integration path proves all of the following in one live-edit
scenario:

1. Initial semantic extraction contains BIM entity/property data for `/World/A`.
2. Applying the recipe does not replace the contextual hierarchy provider.
3. The initial BIM property search `Width = 1` returns `/World/A`.
4. Editing `/World/A.Width` from `1` to `10` emits a `StageChangeBatch` for `/World/A.Width`.
5. `LiveStage` observes the new value.
6. `SemanticSnapshot` contains canonical `Width = 10`.
7. The semantic diff marks `/World/A` modified.
8. BIM read/classification state converges from the old value to the new value.
9. `RequestBimProperties` returns `Width = 10`.
10. BIM search `Width = 10` returns `/World/A`.
11. The contextual hierarchy source and projection remain unchanged.
12. A stale BIM result from generation 1 is dropped after activation generation 2.

## A-H execution matrix

Counts are direct command results and are not added across rows because filters
overlap. Frontend results are preserved from the accepted C9+ matrix because
the frontend is unchanged in this correction batch.

| Row | Contract under test | Actual command(s) | Result |
| --- | --- | --- | --- |
| A | Direct Scene renderer activation, root hierarchy, scene index, and provider wiring | Backend `cargo test --quiet --bin usdview project_and_direct_scene_activation -- --nocapture`; `cargo test --quiet --bin usdview scene_index -- --nocapture` | Preserved 1/1 activation result; current 10/10 scene-index tests passed, including the 40,000-node dense case. |
| B | Project-root presentation, root authority, direct composition, and frontend Projects behavior | Backend `cargo test --quiet --bin usdview migrated_legacy -- --nocapture`; `cargo test --quiet --bin usdview authoring -- --nocapture`; frontend accepted C9+ Projects suite | Preserved backend 2/2 and 15/15 results; frontend preserved 166/166. |
| C | Deep hierarchy ordering/depth, bounded pages, virtual-window inputs, and selection reveal | Current backend scene-index command above; accepted frontend hierarchy/reveal/scale suites | Backend 10/10; frontend preserved 28/28, 10/10, and 5/5. |
| D | BIM search/classification is isolated from contextual hierarchy search | Backend `cargo test --quiet --bin usdview bim -- --nocapture`; backend `cargo test --quiet --bin usdview hierarchy -- --nocapture`; frontend accepted C9+ BIM and hierarchy suites | Backend BIM 49 passed, 1 ignored, 0 failed. Backend hierarchy 12 passed, 1 failed: the inherited `hierarchy_search_routes_through_the_projected_snapshot` assertion at `src/viewport/api/bridge/tests/hierarchy_search_test.rs:234`. Frontend preserved 88/88 BIM and 28/28 hierarchy. |
| E | A -> B -> A activation, acknowledgement, stale response rejection, and UI isolation | Accepted frontend activation/coordinator/scene-cache/M17 suites | Preserved 23/23, 11/11, 13/13, and 10/10. |
| F | Semantic refresh, BIM generation invalidation, dense-index refresh, and package integration | Backend `cargo test --quiet --bin usdview semantic -- --nocapture`; `cargo test -p usd_bevy --quiet`; current scene-index command above | Semantic 88/88 passed; package 132 passed, 1 ignored; scene-index 10/10 passed. |
| G | OR6 storage/portability, direct composition, import closure, localized references, and export/reopen | Accepted C9+ backend portability, authoring, migration, and source-closure commands | Preserved 3/3, 15/15, 2/2, and 15/15. This is CPU-side filesystem/authoring evidence only. |
| H | Animation, Standard4/Extended16-compatible projection, native instances, changed-info, and sparse reconciliation | Accepted C9+ backend animation/native-instance/changed-info/reconcile commands plus current semantic and BIM commands | Preserved 3/3, 15 passed/1 ignored, 4/4, and 12/12; current semantic/BIM correction gates pass. No runtime FPS or native/GPU claim is made. |

## Remaining hierarchy result and provenance

The hierarchy filter is not part of the C6+ production change. Its remaining
failure is deterministic:

```text
viewport::api::bridge::tests::hierarchy_search_test::tests::hierarchy_search_routes_through_the_projected_snapshot
src/viewport/api/bridge/tests/hierarchy_search_test.rs:234:9
assertion failed: run_search(&mut app, "Exterior Wall")?.is_empty()
```

The same test and assertion are present at the pinned C0 commit
`0d98f48e7ec5ba31f8e8421ff18041c73026fd4c`, and the C0 hierarchy adapter
already promoted `PrimNodeReadModel::display_name` to the projected node name.
The current search implementation searches projected names only. This is an
inherited test-contract mismatch, not a C6+ regression, and is recorded here
without changing the accepted C5 hierarchy scope.

## C6+ focused gates

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check --workspace` | Passed; existing warnings only. |
| `cargo check --workspace --no-default-features` | Passed; existing warnings only. |
| `cargo test --quiet --bin usdview m8_semantic_consistency -- --nocapture` | 2/2 passed. |
| `cargo test --quiet --bin usdview semantic -- --nocapture` | 88/88 passed. |
| `cargo test --quiet --bin usdview bim -- --nocapture` | 49 passed, 1 ignored, 0 failed. |
| `cargo test --quiet --bin usdview scene_index -- --nocapture` | 10/10 passed; 40,000-node dense case passed. |
| `cargo test -p usd_bevy --quiet` | 132 passed, 1 ignored, 0 failed. |
| `git diff --check` before C6+ commit | Passed. |

## Evidence boundary

These are local CPU-side compilation, test, and source-provenance results.
They do not prove browser, GPU, native Tauri, WebRTC, Revit, Omniverse,
production, or independent CI behavior. The frontend has no C6+ changes and
there is no artificial frontend checkpoint. Existing unrelated frontend
`Cargo.lock` and `src-tauri/Cargo.lock` edits remain outside this batch.

Owner Review remains required. OR7 is open and OR8 is not started.
