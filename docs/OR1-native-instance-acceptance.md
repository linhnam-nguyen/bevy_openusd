# OR1 — Native OpenUSD Instance Projection Repair

Status: `IMPLEMENTED / OWNER REVIEW 1 READY`

Branch continuity: OR1 continues the current `panel-BIMData` branches. The
backend is `develop/panel-BIMData` at the appended OR1 tip; the sibling UI
workspace remains on `panel-BIMData`. The pre-existing backend history ends at
`6301427`; OR1 was appended without rewriting that history, and no separate
OR1 branch is retained.

Review boundary: Owner Review 1. The `OR1-C9+2` review correction is complete;
no follow-up milestone is started by this packet.

## Scope and root cause

The affected path is the native OpenUSD projection in `usd_bevy`. The Revit
window roots are instanceable references. Their renderable frame and glass
meshes are composed instance-proxy descendants, not children of the synthetic
prototype path. The old projection stopped at the instance root and therefore
did not create scene entities for those proxy meshes.

The repair keeps the scene path as the runtime identity, enables proxy-aware
projection, reuses the existing geometry/material caches, and maintains a
path-based reverse dependency index for prototype edits. Transparent glass
remains transparent by authored opacity; the opaque frame remains renderable.

## Checkpoint ledger

| Checkpoint | Commit | Evidence |
| --- | --- | --- |
| OR1-C1 characterization | `1dea77b` | Real composition-arc fixture proves instance/prototype/proxy behavior. |
| OR1-C2 proxy-aware plan | `6f03bb1` | Scene proxy paths are planned parent-before-child. |
| OR1-C3 shared mesh projection | `4744828` | Proxy mesh entities share mesh and material handles. |
| OR1-C4 presentation semantics | `6a47ecb` | Transforms, visibility, purpose, frame opacity, and glass transparency are covered. `OR1-C9+2` adds leaf `GlobalTransform` proof with shared mesh handles. |
| OR1-C5 reconciliation | `ee1ef72` | Prototype/source edits fan out through the native dependency index; panel suppression export is retained. `OR1-C9+2` adds real-sink structural add/remove, instance removal, and transform-isolation regressions. |
| OR1-C6 nested instances | `aab450e` | Nested instance roots and leaf proxies project without synthetic identity paths. `OR1-C9+2` adds nested prototype resync with an unrelated consumer exclusion. |
| OR1-C7 selection identity | `9b3cc2f` | Scene proxy anchors resolve; prototype paths do not resolve to selectable entities; panel hierarchy tests are retained. `OR1-C9+2` replaces the synthetic map with a rebuild from real projected entities. |
| OR1-C8 Revit/performance audit | `d92d497` | Local `Projet1.usdc` and generated 1,000-instance audits pass. |
| OR1-C9 final acceptance packet | `a728c15` | Required gates and evidence are recorded for Owner Review 1. |
| OR1-C9+ panel branch alignment | `d39389b` | Rustfmt-only merge hygiene after appending OR1 to panel-BIMData. |
| OR1-C9+2 Owner Review 1 correction | `695f3c8` | Real Bevy/OpenUSD runtime regressions close the C4-C7 evidence gaps on the continuous `develop/panel-BIMData` branch. |

## C8 evidence

The explicit local audit command was:

```text
cargo test -p usd_bevy --lib native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
```

Observed result on `develop/panel-BIMData`: 5 Revit window instance roots, 10
instance-proxy meshes, and 10 projected `Mesh3d` entities (`projection_ms`
was 234.97 ms in the correction run). The generated structural audit covers
1,000 instance leaves, 1,000 dependency-index entries, and one shared mesh
asset.

## OR1-C9+2 review-correction evidence

The correction tests author changes through `LiveStage`, retain the real
`PendingStageChanges` batch, and run the production Bevy update schedule:

- C4: proxy leaf `GlobalTransform` resolves to `(-3, 0, 0)` and `(3, 0, 0)`
  for `Window_A` and `Window_B` while both leaves share one `Mesh3d` handle.
- C5: a real prototype structural add spawns both proxy meshes; removal
  despawns both and cleans reverse edges; removing `Window_A` leaves
  `Window_B` and its dependency records unchanged; an authored `Window_A`
  transform does not patch `Window_B` or replace the shared mesh.
- C6: a nested prototype add reaches both nested consumers and does not add a
  proxy under the unrelated `Other_C` instance.
- C7: `SceneAnchorIndex::rebuild()` consumes the projected entities, resolves
  distinct `Window_A/Frame` and `Window_B/Frame` entities, and rejects the
  synthetic prototype path.

Correction commands and observed results:

```text
cargo test -p usd_bevy --lib live::tests::native_instance -- --nocapture
13 passed; 1 ignored

cargo test -p usdview --bin usdview native_instance_selection_resolves_real_projected_scene_proxy_paths -- --nocapture
1 passed

cargo test -p usd_bevy --lib live::tests::native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
1 passed: 5 window roots, 10 proxy meshes, 10 projected Mesh3d entities
```

Changed-info reconciliation queries the native dependency index and patches
only affected scene proxies. Stage-wide traversal remains limited to initial
projection and explicit full reconciliation; subtree resync remains scoped.

## Required gate evidence

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets` | PASS |
| `cargo check --workspace --all-targets --no-default-features` | PASS |
| `cargo test --workspace --all-targets` | PASS |
| `cargo test --workspace --all-targets --no-default-features` | PASS |
| `make harden` | ENVIRONMENT-LIMITED at all-features compile |

`make harden` passed its source-size audit with 0 files over 400 lines and
passed its no-default-features check/test stages. It stopped when
`dlss_wgpu` required an unset `DLSS_SDK`; the same all-features attempt also
reported `wgpu` Vulkan APIs configured out on this host. The harden gate was
not weakened or bypassed.

## Evidence boundary

The tests prove composed-stage discovery, CPU-side Bevy projection, mesh/cache
reuse, reconciliation, and selection identity. They do not prove a live GPU
frame, Tauri/WebRTC delivery, browser playback, or H265 encoding. Those
surfaces remain outside OR1 scope and require a separately capable runtime.
