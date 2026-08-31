# M8-OR2 — BIM Data Acceptance Packet

Date: 2026-08-31

Status: `IMPLEMENTED / OWNER REVIEW 2 REQUIRED — REPAIR BATCH COMPLETE`

Review boundary: Owner Review 2. OR2 is complete at the implementation and
automated-evidence boundary recorded here, including the additive C1+/C3+/C7+
repair batch. It is not marked `PASSED / FROZEN` until the owner reviews this
packet.

## Branch continuity and scope

OR2 is a continuation of the existing panel branches. No separate OR2 branch
was created or retained.

- Backend: `develop/panel-BIMData`
- UI: `panel-BIMData`
- The pre-existing UI `Cargo.lock` modification remains uncommitted and was
  preserved.
- The backend pre-existing `.codex/` directory remains untracked and was not
  staged.

The milestone covers bounded BIM property delivery, stale request handling,
classification color and hierarchy behavior, authoritative field/property
controls, panel flex/scroll behavior, and the integrated acceptance matrix.

## Checkpoint ledger

| Checkpoint | Backend | UI | Result |
| --- | --- | --- | --- |
| M8-OR2-C1 | `0789264` | `243412e` | Bounded BIM property pages/chunks, correlation, reassembly, and explicit oversized-value errors. |
| M8-OR2-C2 | — | `9fc3ba4` | Idle/loading/ready/error lifecycle, supersession, cancellation, and stale-page rejection. |
| M8-OR2-C3 | `2db05a2` | `2164e04` | Typed classification color path, authored-color restoration, Auto refresh, and native-instance occurrence fan-out. |
| M8-OR2-C4 | `533bc75` | `fc43ae8` | Deterministic BIM virtual hierarchy, real scene anchors, `<Unclassified>`, paging, and virtual-group selection boundary. |
| M8-OR2-C5 | — | `0b588c5` | Authoritative field/property controls, validation, revision invalidation, and preserved classification interactions. |
| M8-OR2-C6 | — | `178c816` | Built-in flex/scroll contract, long-value wrapping, compact rows, and source/test module sizing. |
| M8-OR2-C7 | `83972b3` | `178c816` | Initial integrated gates, realistic-load audit, acceptance packet, and Owner Review 2 stop. |
| M8-OR2-C1+ | `dbb99d0` | `a279b12` | Near-linear bounded property-page packing, focused frontend property-state/reducer modules, and 4,096-property scale evidence. |
| M8-OR2-C3+ | `dbb99d0` | — | Indexed scene-occurrence reverse lookup, indexed classification fan-out, and 4,000-occurrence scale evidence. |
| M8-OR2-C7+ | `dbb99d0` | `a279b12` | Repaired source-size/gate rerun, updated packet, continuous-branch pushes, and Owner Review 2 re-opened. |

## C7 implementation and repair evidence

The C7 integration pass made only bounded acceptance-supporting repairs:

- Consolidated BIM property page metadata into a typed `PageContext`, keeping
  the paging helper below the strict argument-count lint and preserving the
  encoded-size boundary.
- Added the public `is_empty` companion required by the native dependency
  index API lint.
- Split the BIM property delivery tests from the chunk test module so the
  source-size audit remains at zero files over 400 lines.
- Added the page/error event arms to the read-model no-op match and repaired
  deterministic classification test reordering for the current map API.
- Replaced per-prefix page cloning and serialization with cached per-item JSON
  sizes plus conservative incremental page accounting. The final page remains
  checked against the authoritative envelope at page boundaries.
- Split the frontend property lifecycle/assembly and property-event reduction
  responsibilities into `properties_state.rs` and `property_reducer.rs`.
- Added `SceneOccurrenceIndex` and routed path-only classification lookup through
  it, so each color entry resolves its actual scene occurrences without scanning
  every indexed entity or allocating a result vector.

### Repair scale evidence

```text
M8-OR2-C1+ paging scale: properties=4096 pages=67 elapsed_ms=90.948
M8-OR2-C3+ indexed color scale: entities=4000 path_lookups=4000 occurrence_visits=4000 elapsed_ms=22.625
```

The C1+ implementation serializes each property and empty-group shape once,
then accumulates bounded size deltas while building pages. The final page is
still validated against the authoritative envelope. The C3+ regression
observes one indexed path lookup and one occurrence visit per unique entry;
the existing duplicate-path regression continues to cover multi-occurrence
fan-out semantics.

## Integrated evidence matrix

### Real supplied asset

Command:

```text
cargo test -p usd_bevy --lib --no-default-features live::tests::native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
```

Observed result:

```text
OR1-C8 Revit audit: window_roots=5 proxy_meshes=10 projected_proxy_meshes=10 projection_ms=244.07
test result: ok. 1 passed; 0 failed; 111 filtered out
```

This is direct CPU-side proof that the windows in `Projet1.usdc` reach
projected `Mesh3d` scene entities. The generated 1,000-native-instance
allocation-bounded audit also passes in both workspace test matrices.

### Backend gates

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets` | PASS |
| `cargo test --workspace --all-targets` | PASS: usd_bevy 111 passed/1 ignored; usdview 337 passed/5 ignored; viewport_streaming 56 passed; remaining workspace tests passed. |
| `cargo check --workspace --all-targets --no-default-features` | PASS |
| `cargo test --workspace --all-targets --no-default-features` | PASS: usd_bevy 111 passed/1 ignored; usdview 337 passed/5 ignored; viewport_streaming 56 passed; all doctests passed or were explicitly ignored. |
| `./scripts/check_rust_file_size.sh` | PASS: 574 files scanned, 0 over 400 lines, 45 warnings in the 351–400 range. |
| Changed-crate strict clippy | PASS for `viewport_streaming`; `usdview` remains baseline-limited by the same four pre-existing findings in untouched API files. No finding was reported for the new packer/index modules. |
| Workspace strict no-default clippy | BASELINE-LIMITED: four findings in untouched `usdview` files (`bim_provenance`, `bim_commands`, and `scene_query`). |
| `make harden` | ENVIRONMENT-LIMITED after source-size and no-default stages: all-features compile requires unset `DLSS_SDK` and reports Vulkan APIs configured out; underlying cargo failure is `bevy_render`, `make` exit 2. |

### UI gates

The repaired UI revision is `a279b12` on `panel-BIMData`; the prior C6 revision
`178c816` remains its parent.

- `cargo fmt --all -- --check`: PASS.
- Focused `usd_hub_desktop` library tests: 228 passed, 1 ignored.
- UI workspace default and no-default compile/test gates: PASS; desktop
  228 passed/1 ignored and viewport client 11 passed in both recorded runs.
- UI source-size audit: repaired `state.rs` 288 lines, `reducer.rs` 349 lines,
  `properties_state.rs` 181 lines, and `property_reducer.rs` 129 lines. Six
  unrelated pre-existing UI files remain over 400 lines and were not touched.
- Strict all-features UI clippy remains baseline-limited: 103 library findings
  and 89 library-test findings in pre-existing files; no finding was reported
  for the new property modules.

## Acceptance coverage

The recorded workspace suites cover the requested stale-request, classification
color, hierarchy, renderer configuration, scroll/panel, idle-activity,
performance, and bounded-delivery surfaces. Representative passing tests
include:

- stale paged-property and stale-property request rejection;
- classification color restoration, repeated Auto refresh, and path-only
  fan-out across native-instance scene occurrences;
- deterministic virtual hierarchy/provider switching and non-selectable
  virtual-group boundaries;
- bounded BIM property paging and explicit oversized-property errors;
- 4,096-property paging scale with 67 bounded pages and no per-prefix rebuild;
- 4,000-entry indexed classification scale with 4,000 path lookups and 4,000
  occurrence visits;
- large BIM fixture cold/idle query, intersection, and color-cost checks;
- renderer configuration, transport, and selection projection acceptance
  tests.

## Evidence boundary

Automated tests prove the authored data path, bounded transport/read-model
behavior, CPU-side projection, hierarchy/classification semantics, and
configuration state transitions. They do not prove a live GPU frame, Tauri
window rendering, WebRTC playback, or H265 output.

Manual visual/runtime evidence is therefore:

```text
UNAVAILABLE — HARDWARE LIMITATION
```

Owner Review 2 should include the supplied launch command in a capable native
GPU/Tauri/WebRTC environment before OR2 is frozen.
