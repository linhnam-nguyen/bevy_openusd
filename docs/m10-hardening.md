# M10 hardening correction packet

M10 is complete and awaiting milestone review. This packet contains the final
corrections requested for C2 through C6. No `Instance2` directory was touched.

## Frozen chain and correction scope

```text
M9 frozen baseline       6b99dfeea5b69e2241d2496b9c89c26b057734b9
M10-C1                   3a56990f0463079d0fe56d5340d875a639d65118
M10-C2                   a8d3fd9213b9bd3269084756367fe0e7d693a153
M10-C3                   005cbae1c9ff624210d3015339edafa8da393d12
M10-C4                   eeef4510a5ac82108bdc1de41a996014f1a63ce4
M10-C5                   013c99cfd05aea8a8ec91c014b6ce9345a605f0c
M10-C6                   4d75d7f73490187d75ed0a485426f43b18a99a51
M10-C2+ through C6+     final correction commit recorded by the handoff
bevy_glacial             424c97b057fc9b9521b020fffa132ee3d022cf6b
UsdHubUI                 f6289b9083d81699bd25857ff5930484756480dc
```

The correction is evidence and gate hardening, plus the persistent runtime
test. It does not change the established renderer, semantic authority, cache
ownership, transport, or frontend architecture.

## C2+ representative runtime comparison

The final comparison uses `Kitchen_set.usdz` at 1920×1080, release profile,
five warmup frames, and 30 measured frames. Both sides passed 16/16 renderer
states and 3/3 cadence states on the same Apple M4 / Metal identity.

| Metric | M9 baseline | Candidate | Delta |
| --- | ---: | ---: | ---: |
| Median CPU frame ms | 3.199 | 3.249 | +1.54% |
| P95 CPU frame ms | 3.510 | 3.635 | +3.57% |
| Actual renderer FPS | 45.045 | 43.383 | -3.69% |
| GPU median / p95 | null / null | null / null | unavailable headless |

The machine-readable comparison is
`target/benchmark/m10-c2-kitchen-comparison.json`; it contains the exact
baseline/candidate Git SHAs, fixture hash, matrix counts, effective state
checks, and the configurable observed-regression value of 3.69%.

## C3+ complete load/edit matrix

`python3 -B scripts/m10_load_edit_matrix.py` produced
`target/benchmark/m10-c3-load-edit-matrix.json` with all eleven required rows:
small, representative, dense, repeated geometry, PointInstancer, transform,
visibility, material, geometry, subtree, and full fallback.

The representative Kitchen projection recorded 2,742 projected prims and
2,743 live-stage prims. The PointInstancer row recorded 40,000 logical
instances and eight unique mesh handles. Transform, visibility, and material
edits performed zero mesh conversions; geometry performed two; subtree
reconciled one spawn and one despawn; full fallback recorded 85 visited prims,
one fallback extraction, one extent recomputation, and one snapshot clone.

## C4+ persistent memory/cache soak

`python3 -B scripts/m10_memory_soak.py --cycles 12` ran one persistent Bevy
`App` for all 12 cycles. The complete artifact is
`target/benchmark/m10-c4-memory-soak.json`, with the runtime detail beside it
at `target/benchmark/m10-c4-persistent-runtime.json`.

The warmed process-tree RSS high-water was 386.83 MiB. Every asset/cache metric was
bounded in the steady half of the run: mesh assets 1463–1464, material assets
4–4, image assets 2–2, projection-cache meshes 1463–1464,
projection-cache sources 1458–1458, material-cache entries 3–3, and
texture-cache entries 1–1. PointInstancer reprojection and twelve resize
generations were recorded in the same process.

## C5+ deterministic gates

The checker now requires the C2 comparison and C3/C4 schemas, validates every
required row and bound, and enforces the shared USDZ fixture's exact
`expected_texture_decode_calls = 1` in both phases. The default universal FPS
floor remains disabled; relative C2 regression limits and machine-specific FPS
floors are opt-in environment/CLI gates.

## C6+ final gates and remaining limits

The required final commands are:

```text
make harden
make bench-render-smoke
```

`make harden` includes formatting, source-size, no-default checks/tests,
strict Clippy, all-feature/all-target tests, and the corrected deterministic
checker. The fresh smoke artifact at
`target/benchmark/m10-c6-render-smoke.json` recorded committed-tip identity,
1920×1080@60 requested/effective state, 49.579 actual renderer FPS, 2.593 ms
median CPU frame time, 3.287 ms p95 CPU frame time, zero grid structural
rebuilds, zero extent scans, zero semantic snapshot clones, and 20 semantic
idle skips. GPU timestamps remain null on the headless path.

GPU timestamps remain unavailable on the headless offscreen path, and RSS is
reported for the cargo/test process tree. These are explicit evidence limits,
not inferred values. No speculative concurrency redesign, frontend workaround,
transport callback mutation, or semantic-authority change was introduced.
