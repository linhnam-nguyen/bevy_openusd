# M10 hardening milestone packet

M10 is complete and ready for review. No `Instance2` directory was touched.

## Pinned chain

```text
M10 base / M9 frozen tip  6b99dfeea5b69e2241d2496b9c89c26b057734b9
M10-C1                    3a56990f0463079d0fe56d5340d875a639d65118
M10-C2                    a8d3fd9213b9bd3269084756367fe0e7d693a153
M10-C3                    005cbae1c9ff624210d3015339edafa8da393d12
M10-C4                    eeef4510a5ac82108bdc1de41a996014f1a63ce4
M10-C5                    013c99cfd05aea8a8ec91c014b6ce9345a605f0c
M10-C6                    this packet's commit tip
bevy_glacial              424c97b057fc9b9521b020fffa132ee3d022cf6b
UsdHubUI                  f6289b9083d81699bd25857ff5930484756480dc
```

The C1 workspace hardening gate passed formatting, source-size, no-default
feature check/tests, strict workspace Clippy, and all-feature/all-target
tests. The all-feature suite included 159 `usdview` tests, 91 `usd_bevy`
tests, 53 streaming tests, protocol compatibility tests, cache profiles,
PointInstancer correctness, progressive loading, and subtree regression tests.
The existing external `bevy_frost` path dependency still emits warnings; it is
not part of this checkout's changes.

## Runtime evidence

The C2 release matrix passed 16/16 renderer-state cases and 3/3 cadence cases
at each resolution on Apple M4 / Metal:

```text
1280x720   actual renderer FPS at 30/60/120 target: 26.88 / 51.71 / 103.42
1920x1080  actual renderer FPS at 30/60/120 target: 26.95 / 51.52 / 102.67
2560x1440  actual renderer FPS at 30/60/120 target: 26.80 / 52.26 / 104.86
```

The matched S1 reports recorded 85 live-stage prims, 14 cached materials, and
15 cached textures. At 1920x1080, median/p95 CPU frame timing was
2.584/3.178 ms. GPU timestamps are explicitly unavailable on the headless
offscreen path and remain `null`, rather than being inferred from CPU timing.

C3 release profiles passed load/edit coverage for repeated geometry, shared
materials and textures, dense geometry, subtree changes, progressive Kitchen
loading, and PointInstancer reprojection. The PointInstancer artifact recorded
40,000 logical instances, eight unique mesh handles, and one material asset;
the live transform reprojection changed transforms without spawning or
despawning instances and kept mesh assets at eight before and after the edit.

C4's warmed process-tree RSS soak passed cache load, PointInstancer edit,
progressive reload, and 1280x720/1920x1080/2560x1440 resize generations. The
overall observed high-water was 1420.45 MiB. Asset/cache bounds were also
verified by the material and instancing artifacts; resize workloads are
separate short-lived processes, so this does not overclaim one-process
long-lived resize retention.

C5's deterministic checker passed all of the following:

```text
python3 -B scripts/check_performance_regressions.py
```

It verifies idle grid/semantic fast paths, positive geometry and grid
transitions, extent/fallback behavior, scoped recovery, texture cleanup,
requested/effective renderer equality, and successful memory soak workloads.
Absolute FPS floors are environment-configurable through
`USDHUB_M10_MIN_RENDERER_FPS`.

## Final commands

```text
make harden
make bench-render-smoke
```

The smoke target runs a fresh release headless S1 report and validates the
idle structural/semantic invariants. The checker is included in `make
harden` so the final workflow cannot silently omit the structural regression
gate. The final smoke report recorded 1920x1080 / 60 FPS requested and
effective, 43.13 actual renderer FPS, 3.040 ms median CPU frame time, zero
grid structural rebuilds, zero extent scans, zero semantic snapshot clones,
and 20 semantic idle skips.

## Architecture and cleanup decisions

- `LiveStage` remains the authoritative USD writer and retained change-batch
  source; no renderer or transport callback was made authoritative.
- Grid structural mesh state remains separate from runtime presentation state;
  idle frames perform neither structural rebuilds nor extent scans.
- Same-session semantic idle work returns before cloning the previous
  snapshot; real edit, recovery, and fallback paths remain covered.
- Mesh/material/texture caches remain distinct and content-addressed; obsolete
  material assets are retired and cleaned after edits.
- Progressive loading keeps one generation, parent-before-child planning,
  cancellation, readiness, and bounded update work.
- PointInstancer uses shared prototype mesh assets with logical instance
  identity preserved; no per-instance mesh duplication was introduced.
- No speculative concurrency redesign, GPU timestamp fabrication, transport
  callback mutation, or frontend workaround was added.

The existing GStreamer pipeline API usage was checked against the official
Rust GStreamer `Element` reference for `link_many` and property-setting
semantics before closing C6.
