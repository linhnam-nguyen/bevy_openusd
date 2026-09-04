# OR8 M2 Projects regression matrix

Status: correction batch implemented; stopped for Owner Review after `M2-C8+3`.

Date: 2026-09-04

## Scope

The test-only harness enters through `ProjectApplicationService`, the same
application/domain service boundary used by the host. It does not add a public
CLI and does not copy lifecycle rules into a second production path.

Each frozen seed runs one complete ordered scenario:

1. create the canonical `Proj_T` fixture;
2. compose fixtures A, B, and C through Model publication or Scene Import/Link;
3. activate three deterministic canonical nested Scenes through the real
   stage/session authority, including shallow and deep hierarchy levels;
4. perform fresh rename, duplicate-name rename, delete, and recreate;
5. select two surviving nested canonical Scenes deterministically after
   mutation, export one, then inspect and reimport it through normal Scene
   adoption;
6. clone the Project filesystem, inspect/import it normally, and validate
   application-local registration identity outside Git-tracked content.

The canonical asset dictionary is rebuilt from `bevy_openusd/assets` and
`Instance2/external_assets` without loading full geometry. The three frozen
fixtures are:

- A: `bevy_openusd:external/PointInstancedMedCity.usdz`;
- B: `bevy_openusd:external/HumanFemale.usdz`;
- C: `external_assets:Omniverse/V1/Projet1.usdc`.

The canonical Project fixture contains eight Scene nodes including the
protected root and exactly this scoped topology:

```text
Proj_T
├── Sc1
│   ├── Sc1.1
│   └── Sc1.2
│       └── Sc1.2.3
└── Sc2
    ├── Sc2.1
    └── Sc1.1
```

The two `Sc1.1` labels remain distinct SceneIds with distinct parents.

Fixture C keeps the exact source USDC bytes and uses a run-owned resolver
support mirror for missing Omniverse MDL/texture resolver inputs. Original
assets are not modified.

Canonical fixture eligibility is based on actual OpenUSD inspection. A is
admitted only when the opened Stage contains a `PointInstancer` or a composed
instanceable prim; B is admitted only when the opened Stage contains a
`SkelAnimation` or authored time samples; and C is admitted only when the
NVIDIA/Revit semantic extractor reports a BIM entity. Package entry names and
bounded text scans remain descriptive metadata for noncanonical assets and
cannot authorize a fixture. A regression fixture named
`PointInstancedMedCity.usda` but containing only an Xform proves the filename
alone is insufficient.

## M2-C8+2 correction boundary

The previous review found that the activation tests resolved a stage and then
manufactured `ProjectActivationReply::activated`, without committing active
stage/session authority. The correction adds a production authority that
admits only a newer command per session and a deterministic test seam that
opens the canonical stage, traverses its hierarchy, extracts the same semantic
snapshot/provider identity, and commits only the exact current command. C4 and
C8 now perform three real transitions with monotonic generations, assert the
active Project/Scene, hierarchy paths, semantic snapshot identity, and reject
an older completion after the third transition. The production Bevy flow also
checks admission before queueing and completion currency before replacing the
LiveStage.

C6 selects a seeded permutation of three nested canonical Scenes. C8 performs
the same selection from surviving nested canonical SceneIds after the lifecycle
delete/recreate step, so the export target is never a stale deleted identity.
Both flows keep export and normal `adopt_scene` as the write path.

## M2-C8+3 correction boundary

Owner Review required the correction to remove the test-only activation shadow
seam, prove empty Project activation cleanup, close the cache-worker lifecycle
race, and use the original BIM source assets. The batch now uses the
production `ProjectStageActivation` candidate and the production Bevy
open-stage installation path in C4/C8. Each transition asserts the actual
`LiveStage`, `StageInfo` generation/path, semantic snapshot and BIM index
generation, hierarchy provider, and projected hierarchy after the third
transition; a late completion from an older generation is rejected.

Empty Project activation clears the old `LiveStage`, stage handle/presentation,
projection cache, semantic snapshot/BIM index, and selection before the empty
authority commit. The cache warmer retains its bounded queue but now exposes
condition-variable quiescence; destructive Project deletion waits for its
Project jobs to finish off the renderer path, and the lifecycle test waits
before removing the cache directory. Inactive Scene deletion snapshots the
active Stage before mutation and asserts the active `SceneRoot` remains
unchanged.

C3/C8 now import/adopt the original `Projet1.usdc` through the normal Scene
Import path. Missing MDL and texture references are classified as optional
rendering-only dependencies and retain authored references for fallback
rendering; no fabricated `OmniGlass.mdl`, `OmniPBR.mdl`, or PNG support files
are used. C8 traces source/target IDs and hierarchy depths, and its sixteen
seeds include both shallow and nested-depth selections.

## Determinism and clean-run policy

The corpus is exactly sixteen seeds:
`0x4F52380000000001` through `0x4F52380000000010`.

Before each seed, the test removes and recreates only its four exact C8
attempt directories under `TestSpaces/OR8/M2/runs`, plus the matching exact
`exports` and `clones` output directories. The Project clone preserves `.git`
and tracked content while excluding only the local derived `.usdhub` cache.
The first attempt is then run from
that clean state. If it fails, the failure trace is written to that attempt’s
`failure.txt`, the directory is retained, and exactly attempts 2, 3, and 4
rerun the same seed from their own clean directories. The test does not retry
until pass.

Failure text includes the seed, attempt, canonical fixture SceneIds, Project
path, deterministic decisions, operation trace, and generated Scene IDs.

## Validation evidence

Final C8 command on the corrected tree:

```text
cargo test --lib or8_m2::c8_tests -- --nocapture
```

Result: 1 test passed, 0 failed, 264 filtered; all 16 seeds completed; elapsed
time 801.70 seconds. Each seed cleared its exact prior attempt/export/clone
directories before starting, and the corrected run retained no final failure
artifacts.

Combined M2 gate on the corrected tree:

```text
cargo test --lib or8_m2 -- --nocapture
```

Result: 14 tests passed, 0 failed, 0 ignored, 251 filtered; the gate includes
the C8 sixteen-seed matrix and the four-seed C1–C7 smoke coverage. Elapsed
time: 824.07 seconds.

The final `make harden` run completed source-size auditing (`804` files,
`83` warning-band files, `0` failure-band files), formatting, diff checks,
workspace checks, and the no-default-feature library/workspace test matrix.
That matrix passed `606` tests, failed `0`, and ignored `5` in `730.09s`,
including the C8 sixteen-seed test and both corrected cache/search regressions.
The gate then stopped at the separate integration test
`profiles_embedded_texture_usdz_fixture`, which observed one USDZ texture
`load_failure` and `304` indexed archive entries while the existing test
expects zero failures and two entries. No cache-profile or archive fixture
source is part of this correction batch. Strict all-feature Clippy/tests and
the performance script were not reached after this inherited fixture/runtime
failure.

Formatting and the focused library and binary compile gates passed. The
source-layout audit has no failure-band file. New files remain below the
200–350-line target, and the five modified warning-band files remain below
the 400-line failure threshold: `source_closure_discovery.rs` (386),
`matrix_persistence.rs` (380), `hierarchy_search_test.rs` (377),
`matrix_steps.rs` (376), and `stage_activation.rs` (361). They are explicitly
reviewable; graph verification, cache ownership, and persistence checks remain
split by responsibility.

The C8 harness and the available desktop self-test surface do not claim GPU,
native Tauri, WebRTC/H265, Revit/Omniverse production, FPS/RAM, or interactive
frontend evidence. No USDHub runtime window was available to the computer-use
self-test. The existing workspace warning set remains unchanged.

## Review boundary

M2 is complete and M3 has not started. Before M3, Owner Review must freeze:

- A: payload storage policy;
- B: CPU/GPU residency budgets;
- C: quantitative performance thresholds.

The exact C8 source commit, C8+ repair commit, and both synchronized plan
records are the checkpoint authority for this report.
