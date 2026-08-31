# M8-OR2 — BIM Data Acceptance Packet

Date: 2026-08-31

Status: `IMPLEMENTED / OWNER REVIEW 2 REQUIRED`

Review boundary: Owner Review 2. OR2 is complete at the implementation and
automated-evidence boundary recorded here, but is not marked `PASSED / FROZEN`
until the owner reviews this packet.

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
| M8-OR2-C7 | `83972b3` | `178c816` | Integrated gates, realistic-load audit, acceptance packet, and Owner Review 2 stop. |

## C7 implementation repairs

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

## Integrated evidence matrix

### Real supplied asset

Command:

```text
cargo test -p usd_bevy --lib --no-default-features live::tests::native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
```

Observed result:

```text
OR1-C8 Revit audit: window_roots=5 proxy_meshes=10 projected_proxy_meshes=10 projection_ms=695.34
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
| `cargo test --workspace --all-targets` | PASS: usd_bevy 111 passed/1 ignored; usdview 331 passed/5 ignored; viewport_streaming 55 passed; remaining workspace tests passed. |
| `cargo check --workspace --all-targets --no-default-features` | PASS |
| `cargo test --workspace --all-targets --no-default-features` | PASS: same relevant suite counts; all doctests passed or were explicitly ignored. |
| `./scripts/check_rust_file_size.sh` | PASS: 570 files scanned, 0 over 400 lines, 46 warnings in the 351–400 range. |
| Changed-crate strict clippy | PASS for `viewport_streaming` and `usd_bevy` with no default features. |
| Workspace strict no-default clippy | BASELINE-LIMITED: four findings in untouched `usdview` files (`bim_provenance`, `bim_commands`, and `scene_query`). |
| `make harden` | ENVIRONMENT-LIMITED after source-size and no-default stages: all-features compile requires unset `DLSS_SDK` and reports Vulkan APIs configured out; underlying cargo failure is `bevy_render`, `make` exit 2. |

### UI gates

The final C6 UI revision remains `178c816` on `panel-BIMData`.

- `cargo fmt --all -- --check`: PASS.
- Focused `usd_hub_desktop` library tests: 228 passed, 1 ignored.
- UI workspace default and no-default compile/test gates: PASS; desktop
  228 passed/1 ignored and viewport client 11 passed in the recorded runs.
- Strict all-features UI clippy remains baseline-limited by 89 findings in
  unrelated pre-existing viewer/benchmark/test files; no C6 files were among
  those findings.

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
