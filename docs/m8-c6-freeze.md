# M8-C6 benchmark freeze

The release gate is `tests/m8_instancing_freeze.rs`. It runs the real
`PointInstancedMedCity.usdz` through `project_stage`, then measures the same
PointInstancer route during reprojection and after a live `positions` edit.

The artifact records logical/visible rows, ECS and render-entity counts, unique
mesh handles, mesh/material asset counts, estimated CPU mesh bytes, projection
times, and `ProjectionCache` lookup/hit/miss totals. Renderer extraction,
draw-batch, and GPU-memory fields are explicitly `null` under `MinimalPlugins`;
they are not inferred from headless timings.

The final artifact is written to `target/m8-c6-instancing-freeze.json` and is
provenance-pinned to the exact commit that ran the release test. Historical
native renderer evidence remains separate from this PointInstancer projection
packet.
