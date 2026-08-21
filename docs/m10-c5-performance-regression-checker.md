# M10-C5+ performance regression checker

The deterministic gate is:

```text
python3 -B scripts/check_performance_regressions.py
```

It now consumes the representative Kitchen baseline/candidate comparison,
the complete eleven-row C3 matrix, and the persistent C4 soak. The older
Hummingbird matrix and frozen M9 incident artifacts remain supplementary
checks.

The checker validates:

- 16 renderer states and three cadence states, with requested/effective state
  equality;
- Kitchen baseline/candidate identity and the recorded relative timing gate;
- zero idle grid rebuilds, extent scans, and semantic snapshot clones;
- positive grid transition, extent recomputation, fallback, and recovery
  evidence;
- transform/visibility/material edits remain mesh-conversion free while
  geometry edits convert mesh data;
- the five initial and six live C3 rows, including PointInstancer logical
  instance count and full fallback extraction;
- one persistent C4 runtime with all asset/cache bounds marked bounded;
- the shared USDZ texture fixture's exact one-decode bound in both initial and
  live phases, plus cleanup;
- no universal absolute FPS floor unless explicitly enabled.

Optional environment-specific gates are opt-in:

```text
USDHUB_M10_MIN_RENDERER_FPS=45 \
USDHUB_M10_MAX_REGRESSION_PERCENT=5 \
python3 -B scripts/check_performance_regressions.py
```

The shared-material artifact now records
`expected_texture_decode_calls = 1`; the checker requires both measured phases
to equal that fixture-specific bound rather than merely being nonzero.
