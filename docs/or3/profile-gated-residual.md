# M8-OR3-C11 profile-gated residual work

Date: 2026-09-01

## Decision

C11 keeps the no-code path for mesh conversion, cache hashing, CPU
parallelism, and streaming copies. The available release profiles show that
the indexed BIM path is already cheap after C8/C9, while no controlled
Hummingbird mesh/GPU/stream profile proves a remaining residual bottleneck.
Adding worker pools, retained scratch buffers, or new frame-copy ownership
without that evidence would add complexity without an attributable benefit.

## Measured evidence

The release BIM profile ran on 4,000 entities and 12,000 properties:

| Measurement | Result |
| --- | ---: |
| classification cold | 32.490 ms |
| classification warm | 3.042 us |
| property-value regex | 3.219 ms |
| first object-search page | 2.506 ms |
| two-target property intersection | 82.625 us |
| classification colors cold | 0.287 ms |
| classification colors warm | 296.958 us |
| classification rebuilds | 1 |

The profiled mesh-builder contract suite passed 3/3 tests in the optimized
build. It verifies source/output accounting and expanded/indexed path
classification, but it does not provide a controlled asset-scale before/after
time. GPU readback and live Hummingbird CPU/RSS/FPS measurements remain
runtime evidence, not unit-test evidence.

## C11 decision table

| Area | Before | After | Benefit | Complexity cost | Decision |
| --- | --- | --- | --- | --- | --- |
| mesh conversion | no asset-scale profile showing it is dominant | unchanged | no demonstrated benefit | none | keep / defer |
| CPU parallelism | no proof that owned mesh conversion dominates | no worker pool | no demonstrated benefit | scheduling, ownership, and join costs | keep / defer |
| cache hashing | no measured hash-dominated run or invalidation proof | unchanged | preserves collision-safe invalidation | none | keep / defer |
| streaming copies | historical evidence did not identify streaming as the animation collapse cause | unchanged bounded queues | no demonstrated benefit | buffer-pool lifecycle and async coupling | keep / defer |
| indexed BIM reads | repeated source scans were replaced by snapshot-owned indexes in C8/C9 | release profile above | measured bounded query path and one classification build | one revision-bound index | keep |

## Gate

```text
cargo test --release --bin usdview \
  viewport::bim::m8_performance_tests::large_bim_fixture_records_cold_idle_query_intersection_and_color_costs \
  -- --nocapture                                      PASS
cargo test --release -p usd_bevy mesh::builder::tests -- --nocapture PASS
```

No C11 code optimization is claimed. The next residual change requires a
controlled before/after profile and must report benefit, complexity cost, and
keep/revert decision before adoption.
