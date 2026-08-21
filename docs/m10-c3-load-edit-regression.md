# M10-C3+ load/edit regression matrix

The correction produces one machine-readable matrix rather than a collection
of unrelated profile claims:

```text
python3 -B scripts/m10_load_edit_matrix.py
```

Artifact: `target/benchmark/m10-c3-load-edit-matrix.json`.

The release run passed all eleven required rows. Initial projection latency was
recorded for every initial fixture:

| Initial row | Fixture | Projection ms | Evidence |
| --- | --- | ---: | --- |
| small | `teapot.usdz` | 160.690 | 16 live-stage prims |
| representative | `Kitchen_set.usdz` | 1047.503 | 2,743 live-stage prims |
| dense | `PointInstancedMedCity.usdz` | 20.921 | 22 live-stage prims |
| repeated geometry | `instanceable.usda` | 0.789 | 8 live-stage prims |
| PointInstancer | `m8_point_instancer.usda` | 25.869 | 40,000 logical instances, 8 unique mesh handles |

The live rows record reconcile latency and work type:

| Live row | Reconcile ms | Mesh conversions | Reconcile evidence |
| --- | ---: | ---: | --- |
| transform | 0.033 | 0 | one patched entity |
| visibility | 0.023 | 0 | one patched entity |
| material | 0.013 | 0 | one patched entity |
| geometry | 0.073 | 2 | two patched entities, two source-cache misses |
| subtree | 0.307 | 0 | one spawn, one despawn, one source-cache hit |
| full fallback | n/a | n/a | 85 visited prims, one fallback extraction, one extent recompute, one snapshot clone |

The matrix consumes the existing release profile/correctness artifacts for
PointInstancer and live mesh patching, while the full fallback row is refreshed
from a current S10 run. This keeps projection, reconcile, conversion, asset,
and frame-impact fields in one schema without claiming frame timing where the
underlying artifact cannot measure it.
