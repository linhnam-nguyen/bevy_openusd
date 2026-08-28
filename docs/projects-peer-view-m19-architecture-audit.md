# M19-C3 Projects Phase-2 architecture audit

This packet records the final Phase-2 Projects review. M19 adds regression
coverage and removes the production fixture fallback; it does not introduce a
new Project product surface, renderer owner, or transport authority.

## Checkpoint ledger

| Repository | Frozen M18 base | M19-C1 | M19-C2 | M19-C3 |
| --- | --- | --- | --- | --- |
| `bevy_openusd` / `develop/project-peerView` | `0d1516e5173b8bf08421792c3fe7458e4bd640c9` | `e158d4a7911c9029780bc57d976b177b79f06ddb` | no backend change required | this commit |
| `UsdHubUI` / `projects-peerView` | `4bf6fafb2ede11d354313a6a7d27678db8e10918` | `dbeb7b0` | `fab6d8b2474d06158c245fdebe73d22b7bda4252` | this commit |

The authoritative implementation plan records the exact final M19-C3 commit
SHAs after both commits are created and pushed.

## M19-C1 regression evidence

Backend `src/project/service/m19_tests.rs` covers the canonical end-to-end
backend path for project creation, unborn `main`, managed local-state roots,
root and nested Scene creation, Model publication into a nested Scene,
activation resolution, and cache/recovery deletion without invalidating the
canonical Project. It also covers native/adopted repository imports and a
moved Project becoming unavailable while another registered Project remains
readable.

UI `apps/desktop/src/features/projects/controller_m19_tests.rs` covers the
deterministic fixture-side interaction matrix: root and nested Scenes, Model
import and Scene composition import, selected-versus-active identity, branch
read/switch behavior, dirty-state protection, overview/Commit intent, and
Viewport opening. Existing M10-M18 tests provide the lower-level native
protocol, stale-completion, root-model restriction, activation, and failure
retention coverage.

## M19-C2 production boundary

`ProjectsController::runtime_default` now always constructs an empty read model
with `TauriProjectsGateway`; it never substitutes the Phase-1 fixture
catalogue. Fixture modules, fixture catalogue access, fixture constructors,
and fixture branch defaults are compiled only under `cfg(test)`. The runtime
read/write path therefore reports the actual gateway result, including host
unavailability, instead of presenting deterministic fixture data as success.

The Projects feature contains no direct component-to-filesystem,
component-to-Tauri, component-to-`gix`, component-to-OpenUSD, or
component-to-renderer-cache path. The platform adapter is the only UI module
that owns the Tauri invocation shape; Project feature code consumes typed
gateway/protocol DTOs and emits typed intents.

## Public Project API inventory

The adapter-neutral `project_protocol` boundary exposes:

- read commands/replies for project lists, Project trees, and repository
  summaries;
- write commands/replies for location inspection, Project creation/import,
  root/nested Scene creation, Scene adoption, and Model import;
- typed Project read/write errors and stable Project/Scene/Model/member
  identities;
- Scene inspection, Model preparation, and import-progress commands/replies;
- Project-to-render-host activation command/reply/result types.

The backend application boundary owns `ProjectId` registry lookup, manifest
identity validation, private repository locators, canonical Stage resolution,
and atomic Project mutations. `ProjectSummary`, `ProjectContentNode`,
`RepositorySummary`, and the placement/member identities are the only Project
read data projected across the host boundary. No filesystem path, Git handle,
OpenUSD Stage, renderer object, or renderer cache key is part of that DTO
surface.

## Queue and ownership audit

| Boundary | Capacity / policy | Owner |
| --- | --- | --- |
| Scene inspection | one worker plus one replaceable pending job | Project service |
| Model preparation | synchronous capacity `4`, bounded prepared-artifact retention | Project service |
| Stage mutation outbox | capacity `128`, rejected as typed `Busy` when full | Project service; applied by existing LiveStage owner |
| Import progress | coalesced retention capacity `64` by `(operation_id, generation)` | Project service progress store |
| Project activation preparation | bounded request/result channels of capacity `2` | existing render-host preparation worker |
| Project-stage activation per UI/session | at most one unresolved activation | existing viewport `SessionCoordinator` |

There is one active-stage authority. Project preparation occurs outside the
Bevy `Update` path, while `LiveStage` remains the owner of activation. The UI
uses generation and stable identity guards for stale replies; it does not add a
second queue or second stage owner.

## Complexity and allocation audit

| Operation | Expected cost | Evidence |
| --- | --- | --- |
| Registry/manifest identity lookup | O(1) average per indexed lookup | `WorkspaceRegistry` and validated manifest indexes use `HashMap` |
| Catalogue list projection | O(P) for P registered Projects | one registry traversal, retaining unavailable entries |
| Project tree projection | O(V + E) for the selected Scene graph | one read and stable placement identities |
| Reusable Scene search | O(V + E) per uncached query | adjacency-indexed traversal; M18 memoization is retained |
| Visible UI rows | O(K) for K visible rows | collapsed descendants are not materialized |
| Atomic publication | O(files changed) plus validation | transaction directory, validate-before-publish, rollback on failure |

The M19 changes do not add hot-path allocations. Project IDs and operation
identities are captured instead of complete records; immutable catalogue
snapshots are shared; worker paths remain private to the backend boundary;
and typed DTOs are owned only at transport boundaries. The M18 allocation and
queue audits remain the governing evidence for renderer/session work.

## Source-layout audit

`./scripts/check_rust_file_size.sh` passes at **580 files, 44 warnings in the
351–400 range, and 0 failures above 400**. The largest materially modified or
new backend file is `src/project/service/mod.rs` at 396 lines; the new
`m19_tests.rs` is 157 lines. The M19 backend diff contains no file above the
400-line hard limit.

The UI M19 diff contains seven Rust files. Their final line counts are:

```text
catalogue.rs 400    controller.rs 378    controller_m19_tests.rs 151
mod.rs 107         model.rs 324        store.rs 148
lib.rs 325
```

The UI repository has unrelated legacy files above 400 lines, but none is
materially modified or newly added by M19. No M19 source-layout warning is
being hidden by a test-only extraction.

## Gate evidence and limitations

- Backend and UI `cargo fmt --all -- --check`: passed.
- Backend `cargo check --workspace --no-default-features`: passed.
- UI `cargo check --workspace`: passed.
- Backend `cargo test --workspace --no-default-features`: passed; the final
  `usdview` unit run was 357 passed and 4 ignored, with all integration suites
  and doctests passing.
- UI `cargo test --workspace`: passed; 293 desktop tests passed, 1 was
  ignored, and 11 viewport-client tests passed.
- Focused M19 backend tests: 2 passed. Focused M19 UI tests: 3 passed.
- `git diff --check`: passed for both repositories.
- `make harden`: format, source-size, no-default check, and no-default test
  stages passed. The all-features stage remains blocked by inherited
  environment/dependency conditions: `DLSS_SDK` is unset, installed
  `wgpu-hal` lacks the Vulkan symbols required by Bevy, and the existing
  `project_protocol::ProjectListItem` `large_enum_variant` Clippy diagnostic
  is promoted to an error by the hardening profile.

These gates are source, compilation, and automated regression evidence. They
do not claim live browser/Tauri, GPU, renderer-hardware, or production
deployment proof. No external web reference was needed for M19; the
authoritative implementation plan and local repository sources are the
references used.

## Known debt and non-goals

- The inherited all-features DLSS/Vulkan hardening blockers remain accepted
  environment debt and are not reclassified as a Projects defect.
- Production branch switching remains an explicit unavailable capability
  until its backend command is authorized; fixture branch mutation is test
  coverage only and cannot silently affect runtime state.
- Commit Graph, Issues/BCF, Team, Turso, renderer-cache ownership, and new
  Frost/Glacial Project branches are outside M19.
- Frost and Glacial were not changed for Projects Phase 2: **NO CHANGE**.

M19 is a freeze/evidence milestone. No M20 work is present in either branch.
