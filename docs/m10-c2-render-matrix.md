# M10-C2+ representative render matrix

The correction reran the required 16-case renderer matrix on
`assets/external/Kitchen_set.usdz` at 1920×1080. The machine-readable paired
comparison is the source of truth; it records the exact baseline and candidate
Git SHAs, scene hash, backend, adapter, requested/effective state, warmup, and
sample settings.

```text
python3 -B scripts/m10_c2_compare.py \
  --baseline-matrix target/benchmark/m10-c2-kitchen-baseline-1920x1080.json \
  --candidate-matrix target/benchmark/m10-c2-kitchen-candidate-1920x1080.json \
  --baseline-s1 target/benchmark/m10-c2-kitchen-baseline-s1-1920x1080.json \
  --candidate-s1 target/benchmark/m10-c2-kitchen-candidate-s1-1920x1080.json \
  --output target/benchmark/m10-c2-kitchen-comparison.json
```

The matched Kitchen comparison passed 16/16 renderer states and 3/3 cadence
states for both M9 baseline `6b99dfeea5b69e2241d2496b9c89c26b057734b9` and the
candidate runtime. With five warmup and 30 measured frames:

| Metric | M9 baseline | Candidate | Delta |
| --- | ---: | ---: | ---: |
| Median CPU frame ms | 3.199 | 3.290 | +2.84% |
| P95 CPU frame ms | 3.510 | 3.891 | +10.87% |
| Actual renderer FPS | 45.045 | 43.585 | -3.24% |
| GPU median / p95 | null / null | null / null | unavailable on headless path |

The comparison reports an observed maximum regression of 10.87%. An optional
relative gate is available through `--max-regression-percent` or
`USDHUB_M10_MAX_REGRESSION_PERCENT`; the universal absolute FPS floor remains
disabled by default.

The supplementary Hummingbird matrix remains available at 720p, 1080p, and
1440p. It still validates requested/effective equality and the idle path, but
the Kitchen comparison above is the representative baseline/candidate proof.

Artifacts:

```text
target/benchmark/m10-c2-kitchen-baseline-1920x1080.json
target/benchmark/m10-c2-kitchen-candidate-1920x1080.json
target/benchmark/m10-c2-kitchen-baseline-s1-1920x1080.json
target/benchmark/m10-c2-kitchen-candidate-s1-1920x1080.json
target/benchmark/m10-c2-kitchen-comparison.json
target/benchmark/m10-c2-1280x720.json
target/benchmark/m10-c2-1920x1080.json
target/benchmark/m10-c2-2560x1440.json
```
