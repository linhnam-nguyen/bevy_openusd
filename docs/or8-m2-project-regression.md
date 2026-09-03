# OR8 M2 Projects regression matrix

Status: implementation complete; stopped for Owner Review.

Date: 2026-09-03

## Scope

The test-only harness enters through `ProjectApplicationService`, the same
application/domain service boundary used by the host. It does not add a public
CLI and does not copy lifecycle rules into a second production path.

Each frozen seed runs one complete ordered scenario:

1. create the canonical `Proj_T` fixture;
2. compose fixtures A, B, and C through Model publication or Scene Import/Link;
3. activate three eligible canonical leaf Scenes through the authoritative
   activation resolver;
4. perform fresh rename, duplicate-name rename, delete, and recreate;
5. export the reserved deep leaf `Sc1.2.3`, then inspect and reimport it through
   normal Scene adoption;
6. clone the Project filesystem, inspect/import it normally, and validate
   application-local registration identity outside Git-tracked content.

The canonical asset dictionary is rebuilt from `bevy_openusd/assets` and
`Instance2/external_assets` without loading full geometry. The three frozen
fixtures are:

- A: `bevy_openusd:external/PointInstancedMedCity.usdz`;
- B: `bevy_openusd:external/HumanFemale.usdz`;
- C: `external_assets:Omniverse/V1/Projet1.usdc`.

Fixture C keeps the exact source USDC bytes and uses a run-owned resolver
support mirror for missing Omniverse MDL/texture resolver inputs. Original
assets are not modified.

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

Result: 1 test passed, 0 failed, 262 filtered; all 16 seeds completed; elapsed
time 604.46 seconds. No final-run failure artifacts remained.

Combined M2 gate after the C8+ clone-race repair:

```text
cargo test --lib or8_m2 -- --nocapture
```

Result: 13 tests passed, 0 failed, 250 filtered. The gate includes the C8
sixteen-seed matrix and the four-seed C1–C7 smoke coverage.

Formatting and compilation passed with `cargo fmt --all` and the C8 no-run
compile gate. The source-layout audit passed: all C8 handwritten Rust files
are within the 200–350 line target; the largest is `matrix_steps.rs` at 348
lines. Graph verification and persistence checks are split by responsibility.

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
