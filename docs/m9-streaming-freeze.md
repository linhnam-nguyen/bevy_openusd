# M9 streaming latency and frame transport — final correction packet

M9 status: **COMPLETE / AWAITING REVIEW**.

This packet supersedes the previously rejected M9 candidate. M10 remains
blocked until the milestone review is complete.

## Pinned revisions

```text
bevy_openusd  caa26d773ad3bda5de40240d1d51bb9b59ab705b
UsdHubUI      f6289b9083d81699bd25857ff5930484756480dc
bevy_glacial  424c97b057fc9b9521b020fffa132ee3d022cf6b
```

The accepted inherited chain remains visible:

```text
M9-C1+    332faca  correlate render identity with readback
M9-C2+    44d9cfe  measure readback and encoder push stages
M9-C3     8fecef3  no change recommended
M9-C4+    96a2010  test production frame-router saturation
M9-C5+    d39d674  require real browser video evidence
M9-C5+++  f446bb6  report incomplete client diagnostics
M9-C1++   c541517  readback-clock PTS trial; rejected by live evidence
M9-C1+++  38a02cf  restore live appsrc media clock
```

The final correction checkpoints are:

```text
bevy_openusd
M9-C1++++ 8fbcf44  bound readback correlation and fail closed on overflow
M9-C2++   4eb4945  report aggregate measured renderer FPS
M9-C4++   5872ba9  split oversized router and benchmark sources
M9-C5++++ 9f6f94c  verify the production-client configuration matrix
M9-C5+++++ caa26d7  cross-check client evidence against server encoder config

UsdHubUI
M9-C4++   2d307f2  split benchmark collector sources
M9-C5++++ f6289b9  capture requested/accepted/applied/decoded stream evidence
```

## C1 — bounded readback identity

`FrameReadbackCorrelation` now has a bounded pending capacity of eight. It
records correlation overflow and high-water telemetry. When the bound is
exceeded it clears pending identities and fails closed until outstanding GPU
readbacks drain; later completions never receive fabricated or mismatched
identities. The live `appsrc` media clock from M9-C1+++ remains authoritative
for media PTS.

Focused coverage proves the overflow path is bounded and does not increment
identity-miss telemetry until the production callback observes the dropped
completion.

## C2 — aggregate renderer cadence

`RendererCadenceSummary.actual_rendered_fps` now uses the measured-window
aggregate `timing.actual_renderer_fps`, not the last instantaneous counter.
Renderer, readback, and encoder-push cadence remain separate values.

The final S12 server report recorded:

```text
requested/effective renderer target: 60 / 60 FPS
actual rendered FPS:                 51.6407
actual readback FPS:                 52.3346
actual encoder-push FPS:             52.3346
readbacks / queued / encoder pushes: 120 / 120 / 120
queue, generation, encoder, and disconnect drops: 0
readback identity misses / overflows: 0 / 0
correlation high-water:              4
```

## C4 — bounded routing and source layout

The production frame router still uses bounded, non-blocking video queues;
reliable control traffic remains independent of video saturation. The router
tests cover a slow encoder, saturation, stale generations, disconnect, and
reconnect.

The changed metrics, router, runner, and benchmark collector modules were
split into focused files. The corrected source files are below 400 lines, and
no visibility was widened to achieve the split.

## C5 — real-client configuration chain

The benchmark runner now executes every requested matrix case and records no
case as supported by silently skipping it. The final matrix artifact is:

```text
target/benchmark/m9-matrix-caa26d7-f6289b9/configuration-matrix.json
```

All nine cases passed through the actual Tauri/WebRTC client:

```text
1280×720   @ 30, 60, 120 FPS
1920×1080  @ 30, 60, 120 FPS
2560×1440  @ 30, 60, 120 FPS
unsupported cases: []
```

Each client artifact proves:

```text
frontend request
  == accepted stream metrics
  == applied Bevy/VideoFrame dimensions and FPS
  == encoder configuration
  == decoded browser dimensions
```

The browser’s measured decoded FPS is allowed to be below the requested FPS.
For example, the 1280×720@120 matrix case decoded at approximately 104.90 FPS,
while the accepted/applied configuration remained 1280×720@120.

The misleading client field `total_delay_ms` was renamed to
`estimated_network_plus_decode_ms`. It represents decoder time plus half the
observed WebRTC RTT; it is not an end-to-end capture-to-display measurement.

## C6 — final release evidence

The exact frozen suite was rerun after all final correction commits:

```text
target/benchmark/m9-final-caa26d7-f6289b9
```

Contents:

```text
24 server reports: S1–S24
7 real client reports: S12–S18
2 Kitchen reports: kitchen-grid-on.json and kitchen-grid-off.json
```

The final S12 client artifact contains:

```json
{
  "decoded": "1920x1080",
  "decoded_fps": 51.94805,
  "estimated_network_plus_decode_ms": 1.9369866,
  "dropped_frames": 0,
  "requested": "1920x1080@60",
  "accepted": "1920x1080@60",
  "applied": "1920x1080@60",
  "completion_blockers": []
}
```

The seven final connected scenarios all decoded 1920×1080 video with zero
drops and empty completion blockers:

| Scenario | Decoded FPS | Measured frames | Requested = accepted = applied |
| --- | ---: | ---: | --- |
| S12 | 51.95 | 104 | 1920×1080@60 |
| S13 | 52.95 | 105 | 1920×1080@60 |
| S14 | 52.00 | 104 | 1920×1080@60 |
| S15 | 52.00 | 105 | 1920×1080@60 |
| S16 | 48.95 | 151 | 1920×1080@60 |
| S17 | 52.00 | 151 | 1920×1080@60 |
| S18 | 53.00 | 105 | 1920×1080@60 |

Both Kitchen variants passed with configuration and steady-state evidence:

```text
kitchen-grid-on:  actual renderer FPS 60.0191
kitchen-grid-off: actual renderer FPS 60.0552
```

## Self-gates

Passed after the final correction commits, including the paired server/client
configuration cross-check:

```text
cargo test -p viewport_streaming                         53 passed
cargo check -p usdview --tests                           passed
cargo test -p usd_hub_desktop                            96 passed
cargo check -p usd_hub_desktop --target wasm32-unknown-unknown passed
cargo check --manifest-path UsdHubUI/src-tauri/Cargo.toml passed
env -u NO_COLOR pnpm frontend:build                      passed
python benchmark syntax check                             passed
real-client S12 configuration matrix                      9/9 passed
real-client S1–S24 release suite                         24/24 passed
Kitchen grid-on/grid-off                                 passed
git diff --check                                          passed
```

Existing GStreamer plugin-scanner warnings, inherited workspace formatting
differences, and inherited lint debt remain visible; none caused a gate
failure. No frontend workaround or `Instance2` path was inspected or
modified.

**M9 status: COMPLETE / AWAITING REVIEW. M10 has not started.**
