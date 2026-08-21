# M10-C3 load and edit regression

M10-C3 completed against backend commit `a8d3fd9` using release-profile tests.

The release gates passed:

```text
cargo test --release --test cache_profile -- --nocapture
cargo test --release --test m8_instancing_correctness --test m8_instancing_profile --test m8_instancing_freeze -- --nocapture
cargo test --release --test progressive_load_profile --test subtree_resync_profile -- --nocapture
```

Coverage and observed evidence:

- repeated mesh, shared material, embedded texture, and Kitchen cache profiles
  passed; the Kitchen run recorded 1,788 lookups, 332 hits, and 1,456 misses;
- material edit evidence kept three material assets bounded and recorded one
  descriptor change plus one retired/cleaned asset;
- PointInstancer coverage passed with 40,000 logical and visible instances,
  eight unique mesh handles, and one material asset;
- live transform reproject passed with one sparse transform patch, zero
  instance spawns/despawns, 40,000 transform updates, and mesh assets bounded
  at eight before and after reproject;
- prototype edits, ancestor resync, selection identity, visibility, and
  logical instance ordering all passed their correctness assertions;
- progressive Kitchen loading passed with one plan build, 86 loading updates,
  2,743 planned work items, and monotonic 25/50/75/100% progress;
- subtree add/remove/material edit coverage passed for the scoped reconcile
  path, with the empirical profile reporting affected-only Bevy and semantic
  work for its synthetic, deep-overlap, and material fixtures.

Current release artifacts include:

```text
target/m6-c5-shared-material.json
target/m7-progressive-load.json
target/m8-c1-instancing-baseline.json
target/m8-c6-instancing-freeze.json
```
