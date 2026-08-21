# M10-C4+ persistent memory/cache soak

The correction replaces short independent subprocess checks with one release
test process owning one persistent Bevy `App`:

```text
python3 -B scripts/m10_memory_soak.py --cycles 12
```

The wrapper records process-tree RSS and embeds the runtime artifact at:

```text
target/benchmark/m10-c4-memory-soak.json
target/benchmark/m10-c4-persistent-runtime.json
```

The run completed 12 cycles in one persistent application, covering load and
reload, transform, visibility, material, geometry, subtree/full reconcile,
PointInstancer reprojection, and 1280×720, 1920×1080, and 2560×1440 resize
generations. RSS high-water was 384.81 MiB for the cargo/test process tree
containing the persistent runtime.

Steady-state bounds were all passed:

| Metric | All-cycle range | Steady-state range |
| --- | ---: | ---: |
| Mesh assets | 4–1464 | 1463–1464 |
| Material assets | 3–4 | 4–4 |
| Image assets | 2–2 | 2–2 |
| Projection cache meshes | 4–1464 | 1463–1464 |
| Projection cache sources | 0–1458 | 1458–1458 |
| Material cache entries | 3–3 | 3–3 |
| Texture cache entries | 1–1 | 1–1 |

The runtime recorded PointInstancer reprojection on three cycles and twelve
distinct resize generations. The RSS scope is stated explicitly: it includes
the cargo/test process tree, not a fabricated allocation-only measurement.
