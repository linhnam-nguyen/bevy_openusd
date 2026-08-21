# M8-C6 benchmark freeze

The release gate is `tests/m8_instancing_freeze.rs`. It runs the real
`PointInstancedMedCity.usdz` through `LiveStagePlugin`, then measures the
ordinary `LiveStage → StageChangeBatch → reconcile → PointInstancerRoute::patch`
path after a live `positions` edit.

The artifact records logical/visible rows, ECS and render-entity counts, unique
mesh handles, mesh/material asset counts, estimated CPU mesh bytes, projection
times, `ProjectionCache` lookup/hit/miss totals, sparse patch count, transform
updates, and instance spawn/despawn counts. The release gate requires 40,000
transform updates with zero instance spawns/despawns and unchanged mesh/material
assets. Renderer extraction, draw-batch, and GPU-memory fields are explicitly
`null` under `MinimalPlugins`; they are not inferred from headless projection
timings. Fresh production-native frame CPU/render-preparation evidence remains
a separate `usdview` release benchmark artifact.

The final artifact is written to `target/m8-c6-instancing-freeze.json` and is
provenance-pinned to the exact commit that ran the release test. Historical
native renderer evidence remains separate from this PointInstancer projection
packet.
