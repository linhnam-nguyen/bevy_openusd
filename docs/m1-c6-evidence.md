# M1-C6 BIM foundation evidence and review stop

Date: 2026-08-27
Repository: `bevy_openusd`
Branch: `develop/panel-BIMData`

## Implemented checkpoints

| Checkpoint | Commit | Result |
| --- | --- | --- |
| M1-C1 | `ea90d1a` | Baseline/source audit recorded |
| M1-C2 | `3b37fd3` | Pure measurement domain types |
| M1-C3 | `8b82332` | Authoritative unit registry/conversions |
| M1-C4 | `1526154` | Explicit evidence-driven extraction mapping |
| M1-C5 | `9a0e42b` | Hashing, diff, persistence, and migrations |

M2 was not started. No frontend, protocol, renderer, LiveStage authoring, or
panel behavior was added.

## Passing evidence

- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `cargo test --workspace --all-targets`: PASS.
- `cargo check --workspace --no-default-features`: PASS through `make harden`.
- `cargo test --workspace --no-default-features`: PASS through `make harden`.
- Focused unit and integration tests cover serde compatibility, all supported
  quantity families, offset temperature conversion, unknown-unit preservation,
  canonical extraction, measurement-sensitive hashes/diffs, working-store
  columns, durable-store columns, and schema migration.
- Source-size audit: PASS, 518 Rust files scanned, 41 existing warnings in the
  351-400 range, 0 failures above 400. The new unit registry is exactly 350
  lines and remains within the repository limit.

## Review-required evidence

`make harden` stops at its strict all-features Clippy stage because the host
environment does not provide `DLSS_SDK`, and the active configuration disables
the Vulkan API required by `bevy_render`. Consequently the all-features test
and performance-regression stages were not reached. This is an environment
gate failure, not a compiler error in the M1 semantic changes.

No real NVIDIA Omniverse Revit Connector export is available in the checkout.
The C4 tests are synthetic contract tests. They prove explicit mapping,
canonical conversion, and no-guess fallback behavior, but they do not prove
connector-version-specific property names or metadata. A real export with BIM
data enabled and its settings recorded is still required before claiming
connector-specific M1 acceptance.

## Freeze status

M1 is implemented and stopped for review, but is not marked `PASSED / FROZEN`
until the reviewer accepts the two evidence limitations above or supplies the
missing fixture/build prerequisites. The implementation branch is clean at
the next checkpoint commit; no M2 work is authorized by this report.
