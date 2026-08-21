# M9 streaming latency and frame transport — correction packet

M9 correction status: **COMPLETE / AWAITING REVIEW**.

This packet supersedes the earlier M9 freeze note. The earlier C6 packet was
not sufficient because its connected run had server-side transport evidence
but no decoded-browser completion artifact. This correction keeps the
renderer, readback, queue, encoder, and client data planes separate and adds
the missing production client proof.

## Effective revisions

Backend (`bevy_openusd`, `develop/optimisation-rendering`):

```text
M9-C1+    332faca  correlate render identity with readback
M9-C2+    44d9cfe  measure readback and encoder push stages
M9-C3     8fecef3  retained from the accepted M9 chain; no new change
M9-C4+    96a2010  test production frame-router saturation
M9-C5+    d39d674  require real browser video evidence
M9-C5+++  f446bb6  report incomplete client diagnostics
M9-C1++   c541517  keep media PTS on the readback clock; runtime check rejected
M9-C1+++  38a02cf  restore the live appsrc media clock
```

The effective backend revision for the final packet is:

```text
38a02cfc5b95192c28a72ba52d7705d29151b30b
```

The final UI revision is:

```text
4aac298446657877e4453a2960f985a7f3e03b2d
```

The Glacial dependency remains pinned to:

```text
424c97b057fc9b9521b020fffa132ee3d022cf6b
```

## Correction results

### C1 — identity and live media timing

Render identity is assigned before readback completion and correlated through
the bounded readback queue. Missing correlations are counted and do not create
a replacement identity after readback. `FrameTrace` remains the source of
sequence and latency metrics.

The first correction attempted to use the readback timestamp as live media
PTS. A real S12 client run still delivered only one video frame, so that
revision was rejected by runtime evidence. `M9-C1+++` restores the live
`appsrc` media clock while retaining the trace for correlation and metrics.
This is the effective transport fix: the browser now receives a continuous
decoded stream.

### C2 — stage accounting

The backend reports renderer cadence separately from readback and encoder
push cadence, and records render→readback, readback→queue,
readback→encoder-queue, readback→encoder-worker, and readback→encoder-push
latencies. The final connected S12–S18 runs each completed 120 readbacks,
120 queued frames, 120 encoder submissions, and 120 pushes. Every connected
scenario reported zero queue-full drops, generation drops, encoder queue drops,
encoder failures, disconnect drops, and readback identity misses.

For S12–S18, renderer cadence was 41.30–55.40 FPS, while measured readback
and encoder-push cadence was 45.68–48.34 FPS. In S12, the stage averages were:

```text
render→readback             44.9775 ms   max 50.4836 ms
readback→queue               0.1842 ms   max  0.4573 ms
readback→encoder queue       0.1955 ms   max  0.5155 ms
readback→encoder worker      0.2001 ms   max  0.5288 ms
readback→encoder push        0.3842 ms   max  0.7240 ms
```

### C4 — bounded production routing

The production `FrameRouter` is exercised with a slow fake encoder. Saturation
is non-blocking and bounded, stale generations are rejected, disconnect and
reconnect are covered, and reliable control traffic remains independent of
video overload. The focused `viewport_streaming` library suite passed all 53
tests.

### C5 — real client completion evidence

The actual UsdHubUI/Tauri/WebRTC client harness now requires a received and
decoded browser video frame with positive dimensions. It records decoded
dimensions/FPS, decode and total delay, RTT/jitter where reported, dropped
frames, and explicit completion blockers. Missing client data is persisted as a
diagnostic sidecar instead of being inferred or silently treated as success.

The effective C1+++ media-clock revision was required for the live client
completion path. The seven S12–S18 client artifacts report:

```text
decoded dimensions       1440 × 960 in every scenario
decoded FPS              46.91–48.00
frames during measure    138–145
dropped frames           0 in every scenario
completion blockers      [] in every scenario
```

### C6 — final release packet

The exact release suite was rerun after the effective backend correction:

```text
python3 scripts/render_bench.py --all --warmup 30 --frames 120 \
  --output-dir target/benchmark/m9-final-c541517-4aac298 \
  --label m9-final-correction \
  --client-command <fresh Trunk + Tauri WebRTC client command>
```

All S1–S24 scenarios passed. The two Kitchen variants also passed:

```text
target/benchmark/m9-final-c541517-4aac298/kitchen-grid-on.json
target/benchmark/m9-final-c541517-4aac298/kitchen-grid-off.json
```

The output directory contains 24 server reports, seven real client reports
for S12–S18, and the two Kitchen reports. S1–S24 remain the authoritative
release matrix; no unavailable client values are inferred for renderer-only or
isolation scenarios.

The final client artifacts are pinned to the backend/UI revisions above. The
server reports include renderer FPS, readback FPS, encoder-push FPS, frame
counts, drop counters, copy/repack counts, and all measured transport stages.
The client reports include decoded browser dimensions/FPS, decode/total delay,
RTT/jitter observations, dropped frames, lifecycle readiness, and blockers.

## Self-gates

Passed before this packet was committed:

```text
cargo test -p viewport_streaming --lib                         53 passed
cargo check -p usdview --tests                                 passed
cargo test -p usd_hub_desktop                                  96 passed
cargo check -p usd_hub_desktop --target wasm32-unknown-unknown passed
cargo check --manifest-path UsdHubUI/src-tauri/Cargo.toml      passed
python3 scripts/render_bench.py --all ...                      S1–S24 passed
Kitchen grid-on/grid-off benchmark                            passed
git diff --check                                               passed
```

The existing GStreamer plugin-scanner warnings and inherited workspace lint
debt remain visible; neither caused a gate failure. No `Instance2` directory
was inspected or modified.

**M9 status: COMPLETE / AWAITING REVIEW. M10 has not started.**
