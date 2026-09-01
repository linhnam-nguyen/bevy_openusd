# M8-OR3-C0 performance baseline

Date: 2026-09-01

## Authorization and revisions

This evidence belongs to the explicitly authorized M8-OR3 C0–C12 pass. C0
adds opt-in counters only; it does not change runtime behavior.

| Repository | Branch | Starting SHA |
| --- | --- | --- |
| `bevy_openusd` | `develop/panel-BIMData` | `5b1810b13e8b64d300065600887ae1a2a70e09cf` |
| `UsdHubUI` | `panel-BIMData` | `e5d58e7cadd595a8e7f5cb2d2ba7328fefe175d5` |

The temporary `aabc5b37` animation checkout remains a behavioral/performance
oracle only and is not a merge source. The existing UI `Cargo.lock` change
and companion `.DS_Store` changes are preserved.

## Counter contract

`usd_bevy::PerformanceCounters` is an opt-in resource. It records the
animation, projection, reconciliation, semantic, scene, and hierarchy values
required by OR3. The default-disabled resource keeps normal sessions at a
single branch per recording call and is resettable without changing its
enabled state.

The current baseline is a structural baseline: the counters are installed and
the first projection/reconciliation/animation route boundaries record their
work. Later checkpoints must populate the remaining counters where their
optimized path owns the relevant operation.

## Automated baseline gate

Backend:

```text
cargo check --workspace --all-targets --message-format=short    PASS
```

Frontend:

```text
cargo check --workspace --all-targets --message-format=short    PASS
```

The frontend check emitted inherited dead-code warnings only. The backend
check emitted inherited Frost warnings only.

`cargo test`, release runtime FPS/CPU/RSS measurements, GPU readback, native
window evidence, and equivalent prepass/shadow Hummingbird comparison remain
open evidence for C12. No values are fabricated here.

## C0 scope result

- [x] exact starting SHAs recorded;
- [x] opt-in counter resource added;
- [x] stage-time, animation, projection, and reconciliation boundaries wired;
- [x] baseline workspace checks passed;
- [ ] runtime and scale measurements, deferred to the integrated matrix.
