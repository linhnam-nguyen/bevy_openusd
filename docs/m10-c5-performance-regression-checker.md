# M10-C5 performance regression checker

The deterministic checker is:

```text
python3 -B scripts/check_performance_regressions.py
```

It passed against the fresh M10 C2/C4 artifacts plus the frozen M9 and
material/mesh correction artifacts. It checks:

- all 16 renderer states and all three cadence states at each C2 resolution;
- requested/effective renderer equality;
- idle grid fast-path behavior: no extent scan or structural rebuild;
- same-session semantic idle skips with zero snapshot clones;
- positive grid-style structural rebuild evidence;
- positive transform/geometry edit evidence, while transform-only edits remain
  sparse;
- edit-time extent recomputation and the intended full fallback;
- recovery checkpoint plus scoped subtree extraction without an unexpected
  fallback;
- repeated shared-texture phase lookup and cleanup;
- six successful C4 soak workloads with RSS high-water values.

Absolute FPS floors are intentionally environment-configurable. The default is
no floor; a machine-specific gate can be enabled without changing the checker:

```text
USDHUB_M10_MIN_RENDERER_FPS=45 python3 -B scripts/check_performance_regressions.py
```
