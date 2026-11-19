# Dormant projected root tests

This directory contains 30 remaining quarantined test files (37 test functions) under `tests/dormant/projection` whose full contracts remain unresolved (`DEFER_WITH_REASON`) due to reported runtime/schema gaps or incomplete evidence of equivalent coverage. Seven fully resolved files remain archived and removed during repository cleanup (`NEM-CLN-001`); nine originals were restored after Reviewer findings. All existing active ports are retained.

The repository-local [AUDIT.md](AUDIT.md) is the current per-file index for the 30 remaining quarantined files, recording test names, fixtures, reported blockers, partial ports where applicable, and file identity.

## Registration and attachment status

Root `Cargo.toml` sets `autotests = false` and declares explicit `[[test]]` targets. None of the 30 remaining files is registered or attached via `#[path]`.

## Remaining deferral basis

Each remaining file retains at least one unresolved assertion, either because of incomplete equivalence evidence, reported missing runtime projection/schema support or an upstream `openusd` parser/interpolation gap (such as Hermite/NURBS/TetMesh routes, double-sided materials, light linking, `read_asset_info`, `UsdPhysicsScene` / articulation / unit conversion, `RenderProduct`/`RenderVar`, `UsdKind` component projection, integer-literal float `timeSamples` parsing, or per-attribute `interpolation = "held"` evaluation).

The restored originals preserve historical geometry, purpose visibility, entity naming, variant discovery, skinning metadata, subdivision inventory, color-space and native-instance contracts. Current active tests cover supported subsets only; retention is not activation or an assertion that the old API works.
