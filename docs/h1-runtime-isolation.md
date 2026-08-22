# H1 — Runtime isolation and regression hardening

H1 is the post-M10 hardening milestone for the frozen rendering/data-plane
architecture. It does not reopen M1–M10 optimization decisions.

## Checkpoint chain

```text
H1-C1  classify semantic paths and instrument PostUpdate
H1-C2  capture Kitchen native/WebRTC PostUpdate baseline
H1-C3  isolate runtime BlobStore/delivery filesystem work
H1-C4  isolate recovery serialization/filesystem work
H1-C5  bound semantic state/query mailboxes
H1-C6  couple delivery publication to projection readiness
H1-C7  final regression matrix and freeze
```

## Runtime invariants

- `LiveStage` and OpenUSD remain owner-thread-only.
- Bevy `Update`/`PostUpdate` performs only owner-thread extraction,
  serialization, immutable descriptor creation, and bounded submission.
- BlobStore, runtime-delivery, recovery, and worker result queues are bounded.
- A saturated semantic state lane recovers with the latest complete snapshot;
  it never silently drops an arbitrary delta.
- Runtime delivery publishes only for the current session, live revision,
  projection generation, and `ProjectionReadiness::Ready`.
- Worker threads receive owned data only. They do not access `World`, ECS
  assets, `LiveStage`, or OpenUSD stages.

## Reproducible evidence

H1-C2 captured the pre-change baseline with:

```bash
python3 scripts/capture_h1_baseline.py
```

H1-C7 repeats the same Kitchen S1 native and S11 headless-WebRTC cases and
records before/after timing and telemetry in:

```text
target/benchmark/h1-c7-regression/regression.json
```

The final hardening gates are:

```bash
python3 scripts/capture_h1_c7.py
make harden
make bench-render-smoke
```

`make harden` includes the workspace format/check/test/Clippy gates, source
size audit, and inherited M10 deterministic regression checker. Real-client
S12–S18 evidence remains the frozen M10 packet because H1 makes no frontend
changes; the packet records that provenance explicitly.

## Acceptance

H1 is accepted only when the checkpoint chain is compile/syntax-clean, source
size is within budget, fresh C7 native/WebRTC artifacts pass their identity and
steady-state checks, and the M10 hardening gates pass. Any inherited warnings
or unavailable headless GPU timestamps remain visible as evidence limits.
