# M10-C4 memory and cache soak

M10-C4 completed against backend commit `005cbae` with the warmed release
soak runner:

```text
python3 -B scripts/m10_memory_soak.py
```

The runner samples the RSS of each workload process and all descendants using
`ps`, then writes `target/benchmark/m10-c4-memory-soak.json`. All six
workloads exited successfully:

| Workload | High-water RSS |
| --- | ---: |
| cache-load-kitchen | 484.58 MiB |
| point-instancer-edit | 274.44 MiB |
| progressive-reload | 348.12 MiB |
| resize 1280×720 | 1412.31 MiB |
| resize 1920×1080 | 1086.86 MiB |
| resize 2560×1440 | 1420.45 MiB |

The overall warmed process-tree high-water was 1420.45 MiB. The first run was
discarded because it included release compilation; the recorded artifact is
the repeated warmed run. Render resizing uses separate short-lived benchmark
processes, so the result is a peak-per-generation observation, not a claim
that one long-lived process was retained across every resize.

Cache and obsolete-asset bounds are covered by the same release run’s
artifacts and tests: Kitchen cache counts are finite, the PointInstancer
reproject keeps eight mesh assets before and after the edit, and the shared
material edit retires and cleans one replaced asset while keeping three live
material assets.
