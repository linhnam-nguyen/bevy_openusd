# OR8 M2 Projects regression matrix

Status: correction batch implemented; stopped for Owner Review after `M2-C8+2`.

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

Final C8 command:

```text
cargo test --lib or8_m2::c8_tests -- --nocapture
```

Result: 1 test passed, 0 failed, 264 filtered; all 16 seeds completed; elapsed
time 550.88 seconds. The first exploratory run retained expected failure
artifacts for seed `0x4F52380000000003` because it exposed the deleted-target
bug; the corrected clean rerun passed and did not retain final-run failures.

Combined M2 gate after the C8+2 correction:

```text
cargo test --lib or8_m2 -- --nocapture
```

Result: 14 tests passed, 0 failed, 0 ignored, 251 filtered; the gate includes
the C8 sixteen-seed matrix and the four-seed C1–C7 smoke coverage. Elapsed
time: 583.68 seconds.

The required `make harden` gate completed its source-size audit (`800` files,
`85` warning-band files, `0` failure-band files) and formatting/check stages,
then stopped at the no-default-feature workspace test stage. That stage ran
`263` tests successfully and reported two inherited failures:
`project::service::m19_tests::phase2_freeze_matrix_covers_create_import_composition_and_recovery`
(`DirectoryNotEmpty`) and
`project::service::stage_mutation::tests::deleting_an_inactive_scene_is_consumed_without_mutating_the_active_stage`
(`SceneRoot` assertion), in `651.85s`. The OR8 M2 C8 test passed in the same
run. Strict all-feature Clippy/tests and the performance script were not
reached after this required gate failure.

Formatting and the focused library and binary compile gates passed. The
source-layout audit has no failure-band file: new `asset_inspection.rs` is 230
lines and `project_activation_flow.rs` is 203 lines. Modified warning-band
files are `stage_activation.rs` (379), `matrix_steps.rs` (381), and
`matrix_persistence.rs` (365); they remain below the 400-line failure
threshold and are explicitly reviewable. Graph verification and persistence
checks remain split by responsibility.

The C8 harness does not claim GPU, native Tauri, WebRTC/H265, Revit/Omniverse
production, FPS/RAM, or interactive frontend evidence. The existing workspace
warning set remains unchanged.

## Review boundary

M2 is complete and M3 has not started. Before M3, Owner Review must freeze:

- A: payload storage policy;
- B: CPU/GPU residency budgets;
- C: quantitative performance thresholds.

The exact C8 source commit, C8+ repair commit, and both synchronized plan
records are the checkpoint authority for this report.
