# H1-C2 — Kitchen PostUpdate baseline

H1-C2 records the pre-worker baseline for the two server compositions that can
be reproduced without a live browser session:

- S1: native headless composition;
- S11: headless WebRTC server composition.

Both cases use `assets/external/Kitchen_set.usdz`, release mode, 1920×1080,
60 FPS, and the same warmup/measurement frame counts. The capture command is:

```bash
python3 scripts/capture_h1_baseline.py
```

The script runs the existing benchmark harness, validates the effective
configuration and steady-state invariants, and writes the machine-readable
packet to `target/benchmark/h1-c2-baseline/baseline.json`. The two source
reports retain the complete semantic/grid telemetry used by H1-C7 for the
before/after comparison.

This checkpoint deliberately does not claim real-client proof. S12–S18 require
the UsdHubUI harness and remain part of the final H1-C7 regression matrix.
