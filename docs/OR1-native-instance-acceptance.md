# OR1 — Native OpenUSD Instance Projection Repair

Status: `IMPLEMENTED / REVIEW REQUIRED`

Review boundary: Owner Review 1. No follow-up milestone is started by this
packet.

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
| OR1-C1 characterization | `12faa56` | Real composition-arc fixture proves instance/prototype/proxy behavior. |
| OR1-C2 proxy-aware plan | `2f4c2c7` | Scene proxy paths are planned parent-before-child. |
| OR1-C3 shared mesh projection | `acfbf49` | Proxy mesh entities share mesh and material handles. |
| OR1-C4 presentation semantics | `f8be706` | Transforms, visibility, purpose, frame opacity, and glass transparency are covered. |
| OR1-C5 reconciliation | `21c3155` | Prototype/source edits fan out through the native dependency index. |
| OR1-C6 nested instances | `704baea` | Nested instance roots and leaf proxies project without synthetic identity paths. |
| OR1-C7 selection identity | `9335c40` | Scene proxy anchors resolve; prototype paths do not resolve to selectable entities. |
| OR1-C8 Revit/performance audit | `6bfba21` | Local `Projet1.usdc` and generated 1,000-instance audits pass. |

## C8 evidence

The explicit local audit command was:

```text
cargo test -p usd_bevy --lib native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
```

Observed result: 5 Revit window instance roots, 10 instance-proxy meshes, and
10 projected `Mesh3d` entities. The generated structural audit covers 1,000
instance leaves, 1,000 dependency-index entries, and one shared mesh asset.

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
