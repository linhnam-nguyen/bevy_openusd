# M10-C2 render matrix

M10-C2 completed on 2026-08-21 from backend commit `3a56990`.

The release renderer matrix ran all 16 renderer-state combinations and all
three cadence targets at each resolution. Every report passed with
`cases=16`, `cadence_samples=3`, and matching requested/effective state.

| Resolution | Matrix | Actual renderer FPS at 30/60/120 target | S1 median / p95 frame ms | Scene prims | Materials / textures |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1280×720 | 16/16 | 26.88 / 51.71 / 103.42 | 2.745 / 3.002 | 85 | 14 / 15 |
| 1920×1080 | 16/16 | 26.95 / 51.52 / 102.67 | 2.584 / 3.178 | 85 | 14 / 15 |
| 2560×1440 | 16/16 | 26.80 / 52.26 / 104.86 | 2.601 / 3.027 | 85 | 14 / 15 |

The runs used the Hummingbird USDZ fixture on Apple M4 / Metal, with five
warmup frames and 30 measured frames. The headless offscreen path does not
publish GPU timestamp timings, so `gpu_median_frame_ms` and
`gpu_p95_frame_ms` are explicitly `null`; CPU/wall timing remains recorded.

Representative idle invariants at all three resolutions were:

- `grid.structural_rebuilds = 0` and `grid.compute_extent_calls = 0`;
- `semantic.idle_skips = 30` and `semantic.snapshot_clones = 0`;
- requested/effective renderer configuration matched;
- steady-state expectation matched.

Machine-readable artifacts:

```text
target/benchmark/m10-c2-1280x720.json
target/benchmark/m10-c2-1920x1080.json
target/benchmark/m10-c2-2560x1440.json
target/benchmark/m10-c2-s1-1280x720.json
target/benchmark/m10-c2-s1-1920x1080.json
target/benchmark/m10-c2-s1-2560x1440.json
```
