# M9 streaming latency and frame transport

M9 is the streaming transport milestone. Its implementation keeps renderer
cadence and media cadence observable as separate data planes, carries frame
identity through readback and encoding, and keeps video backpressure bounded
without sharing the reliable command path with frame traffic.

## Checkpoint commits

```text
M9-C1  63abb28934d676294ef30adc312fc36ce50799b7
M9-C2  4a8b4f0c0c5fdc4484112953aa5b9cdf537721dd
M9-C3  8fecef3eb664085939778d8e387926daba1cd6ce
M9-C4  7ab7b82acc1d73d2cb634552f4bd1f63f6d2fd6c
M9-C5  1d88795ea015012a73f32f974467d3bd7844bc29
M9-C6  <this documentation and freeze commit>
```

The M9 source changes are confined to the viewport transport and streaming
crates, their diagnostics/report wiring, and the focused configuration tests.
The C6 commit contains the milestone packet and plan update; it does not
change the runtime implementation after the C5 gates.

## Implementation result

### C1 — frame identity and timestamps

`VideoFrame` now carries a monotonic `FrameTrace` containing a sequence number
and process-relative timestamp, in addition to dimensions and generation.
The trace is preserved from readback through the bounded frame queue and into
the encoder. Encoder buffers receive the source timestamp as PTS and retain
the configured duration. Pixel ownership is moved into `Arc<Vec<u8>>` so the
transport can share the completed frame without an additional full-frame
conversion.

### C2 — capture and drop accounting

`FrameTransportMetrics` records readback completion, capture, queueing,
queue-full drops, invalid readbacks, repacking/copy bytes, generation drops,
encoder submission/push/failure, disconnect drops, and capture-to-queue and
capture-to-encoder latency. The metrics are included in the performance report
without changing the existing drop policy.

### C3 — readback copy gate

Aligned readback rows keep the original `Vec<u8>`. Padded rows are compacted
in place with `copy_within` and truncated, avoiding a second full-frame
allocation. Focused tests cover aligned rows, padded rows, and capacity
behavior. No unsafe zero-copy path was introduced without an ownership proof.

### C4 — bounded render/readback/encode overlap

Each active video target owns a bounded `sync_channel(2)` and a named encoder
worker. Enqueue is non-blocking; a saturated or disconnected video target
drops only video work and increments the corresponding transport metric.
Generation checks run both before enqueue and in the worker. Worker teardown
joins before session encoder shutdown. Reliable data/control channels remain
separate and are not dropped because video is overloaded. Focused tests cover
queue saturation, disconnect, generation mismatch, and reconnect behavior.

### C5 — configuration matrix

The streaming test suite validates raw GStreamer caps for:

```text
1280×720, 1920×1080, 2560×1440
30, 60, 120 FPS
```

The matrix test checks the actual caps structure fields used by the pipeline.
Runtime codec coverage remains limited to codecs available in the executing
GStreamer installation.

## Runtime benchmark packet

The generated artifacts are local ignored benchmark outputs and are not source
files. They are retained at:

- [`target/m9-s11.json`](../target/m9-s11.json)
- [`target/m9-s12-long.json`](../target/m9-s12-long.json)

Both artifacts run the M9-C5 source revision
`1d88795ea015012a73f32f974467d3bd7844bc29`, with Glacial pinned to
`424c97b057fc9b9521b020fffa132ee3d022cf6b`, on macOS Metal / Apple M4 at
1920×1080 and requested 60 FPS.

### S11 — renderer-only control

This run intentionally had no connected video encoder. Across 120 measured
frames:

```text
median / p95 CPU frame       19.5505835 / 20.9658229 ms
actual renderer FPS          48.6072
readback completions         120
queued frames                120
queue-full drops             0
invalid readbacks            0
generation drops             0
encoder submissions/pushes   0 / 0
disconnect drops             120
capture→queue avg / max      0.0087747 / 0.014334 ms
```

The disconnect count is expected for a renderer-only run and is not reported
as an encoder failure.

### S12 — server-side connected transport

The Tauri/WebRTC launch was exercised with an explicit Trunk server and the
server-side report completed. The external client harness did not finish, so
this is transport/encoder evidence, not decoded-browser proof. Across 120
measured frames:

```text
median / p95 CPU frame       19.711354 / 20.4794982 ms
actual renderer FPS          47.8822
encoded FPS                  60
readback completions         120
queued frames                120
queue-full drops             0
invalid readbacks            0
generation drops             0
encoder submitted/pushed     120 / 120
encoder queue drops/failures 0 / 0
disconnect drops             0
capture→queue avg / max      0.00895765 / 0.016375 ms
capture→encoder avg / max    0.02215661 / 0.051708 ms
repacked frames              120
```

No frontend FPS, decoded-video dimensions, browser decode latency, RTT, or
jitter claim is made from this run because the client harness did not produce
its completion artifact. That limitation is explicit for milestone review.

The GStreamer test run emitted the environment's existing GLib/GTK plugin
scanner warnings while the focused caps test still passed. They did not
produce a Rust test failure.

## Self-gates

The checkpoint gates passed as follows:

```text
M9-C1  cargo check -p viewport_streaming --tests
      cargo test -p viewport_streaming frame_metrics -- --nocapture
      cargo check -p usd_bevy --tests

M9-C2  cargo check -p usd_bevy --tests
      cargo test -p viewport_streaming frame_metrics -- --nocapture

M9-C3  cargo check -p viewport_streaming --tests
      cargo test --bin usdview viewport::transport::frame_capture -- --nocapture
      cargo check -p usdview --tests

M9-C4  cargo check -p viewport_streaming --tests
      cargo test -p viewport_streaming --lib
      cargo check -p usdview --tests

M9-C5  cargo test -p viewport_streaming encode::tests::raw_caps_preserve -- --nocapture
```

The final C6 gate reruns the focused streaming tests, the viewport transport
tests, the `usdview` test compile, `rustfmt --check` for the changed Rust
files, and `git diff --check` before committing this packet.

No frontend checkout or `Instance2` path was touched.

**M9 status: COMPLETE / AWAITING REVIEW.**
