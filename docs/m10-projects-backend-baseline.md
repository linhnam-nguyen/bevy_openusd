# Projects M10-C1 backend baseline

## Checkpoint

```text
milestone: M10-C1
repository: bevy_openusd
branch: develop/project-peerView
source branch: server/develop
BACKEND_ENGAGEMENT_BASE_SHA: 68d3deb4645b030a653f5b27beb514c68073180e
PROJECTS_UI_ADAPTED_FROZEN_SHA: 3b74dd5aa78628e44833e0877259c5122cb6d897
```

The derived branch was created from the clean, synchronized
`server/develop` tip above. No backend Project code was present before this
checkpoint.

## Baseline gate

The required `make harden` baseline was run without changing unrelated root
behavior.

```text
git diff --check: PASS
cargo fmt --all -- --check: PASS
source-size audit: PASS (515 files, 41 warning-band files, 0 failures)
cargo check --workspace --no-default-features: PASS
cargo test --workspace --no-default-features: PASS
```

The gate stopped at the all-features Clippy stage because of inherited local
environment/toolchain requirements:

```text
cargo clippy --workspace --all-targets --all-features -- -D warnings:
  BLOCKED / FAIL at environment-dependent all-features stage
```

Observed blockers:

- `DLSS_SDK` is not set, so `dlss_wgpu`'s build script aborts;
- the installed `wgpu-hal` configuration has Vulkan disabled, so Bevy's
  `raw_vulkan_init.rs` cannot resolve `wgpu::hal::api::Vulkan` or
  `wgpu::hal::vulkan` (`E0432`/`E0433`).

Because Clippy stopped `make harden`, the later all-features test and
performance-checker stages were not executed by that command. This is a
baseline environment limitation, not a Project-domain implementation failure.

## Scope and next checkpoint

M10-C1 records the backend engagement baseline only. It does not repair the
inherited DLSS/Vulkan environment and does not add Project behavior.

The next checkpoint is:

```text
M10-C2: add pure Project domain identities
```

STOP at the M10 milestone review boundary after M10-C4; do not begin M11
without review approval.
