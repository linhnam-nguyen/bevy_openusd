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
| small | `teapot.usdz` | 161.377 | 16 live-stage prims |
| representative | `Kitchen_set.usdz` | 1046.619 | 2,743 live-stage prims |
| dense | `PointInstancedMedCity.usdz` | 21.277 | 22 live-stage prims |
| repeated geometry | `instanceable.usda` | 0.725 | 8 live-stage prims |
| PointInstancer | `m8_point_instancer.usda` | 25.869 | 40,000 logical instances, 8 unique mesh handles |

The live rows record reconcile latency and work type:

| Live row | Reconcile ms | Mesh conversions | Reconcile evidence |
| --- | ---: | ---: | --- |
| transform | 0.062 | 0 | one patched entity |
| visibility | 0.114 | 0 | one patched entity |
| material | 0.161 | 0 | one patched entity |
| geometry | 0.458 | 2 | two patched entities, two source-cache misses |
| subtree | 3.239 | 0 | one spawn, one despawn, one source-cache hit |
| full fallback | n/a | n/a | 85 visited prims, one fallback extraction, one extent recompute, one snapshot clone |

The matrix consumes the existing release profile/correctness artifacts for
PointInstancer and live mesh patching, while the full fallback row is refreshed
from a current S10 run. This keeps projection, reconcile, conversion, asset,
and frame-impact fields in one schema without claiming frame timing where the
underlying artifact cannot measure it.
