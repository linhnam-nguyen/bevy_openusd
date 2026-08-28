# M19-C3+++ Projects Phase-2 architecture audit

This packet records the final M19 repair evidence. M19 freezes the existing
Projects Phase-2 workflow and production boundary; it does not add another
product surface, renderer owner, or transport authority.

## Checkpoint ledger

| Repository | Frozen M18 base | M19-C1 | M19-C2 | M19-C3 | M19-C1++ | M19-C3++ | M19-C1+++ | M19-C3+++ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `bevy_openusd` / `develop/project-peerView` | `0d1516e5173b8bf08421792c3fe7458e4bd640c9` | `e158d4a7911c9029780bc57d976b177b79f06ddb` | no backend change required | `9295c63c2cf6fdc2f1e798370acc41d606f9927b` | `4e18fabdf6515455bb0adce34f37bb7712fc0414` | `8f3d9ee3fc525a784ec44bbf7f22bab97b13192e` | `f75fea096134c04d6cf9e431972243c403760fb0` | this commit |
| `UsdHubUI` / `projects-peerView` | `4bf6fafb2ede11d354313a6a7d27678db8e10918` | `dbeb7b0922b4c9ce06ca7751186a945313fecf6a` | `fab6d8b2474d06158c245fdebe73d22b7bda4252` | `c63f8dd54a5f24c413dfb8d735e57f4ec4ddb16d` | `8c6d479ccfcfabc266850caeb33f5fddf8c29d1c` | `57739cd09789a7eb4e36e3c2dfac9aab59cb0e8e` | `617793d215e0b6ad3dd05481d9f32c6edf83fff6` | this commit |

The implementation-plan ledger records the exact M19-C3+++ tip SHAs after the
audit-packet commits are pushed.

## Final repair contract

Production branch switching is now backend-owned and Git-neutral. The service
performs dirty-worktree preflight, checks out the requested existing local
branch without stash, discard, force, or rollback, then validates that
branch's Project metadata.

If checkout succeeds but any target Project projection is invalid, the service
returns `BranchProjectInvalid` carrying the actual post-checkout
`RepositorySummary` whenever repository truth is readable. If the manifest is
valid but a Scene/Model projection or repository summary cannot be read, it
returns the distinct `BranchProjectTruthUnavailable` signal. The repository
remains on that branch so the user can recover explicitly. The UI replaces
repository truth with the structured summary when available; otherwise it
invalidates the old repository and tree authority, marks Project content
unavailable, and keeps the diagnostic. Generation and ProjectId guards still
reject stale completions.

The Git adapter also treats ignored `.usdhub/cache` and `.usdhub/recovery`
files as local implementation state rather than user edits. Ignored-only
changes remain clean and do not block an otherwise eligible branch switch.

## M19 regression evidence

Backend `src/project/service/m19_tests.rs` retains the end-to-end Project
workflow coverage for creation, unborn `main`, managed local-state roots,
root and nested Scene creation, Model publication, activation resolution,
cache/recovery deletion, repository imports, and moved-Project isolation.

The M19 repair adds:

- `usd_git` coverage for ignored cache/recovery state, dirty and untracked
  protection, valid switching, and no stash/discard behavior;
- Project-service coverage proving an invalid target branch reports the actual
  checked-out branch and remains recoverable by switching to a valid branch;
- Project-service coverage proving a corrupt Scene document after checkout
  cannot surface the old branch as authoritative;
- UI coverage proving an invalid target branch cannot keep the old Scene/Model
  hierarchy authoritative while the actual branch summary is displayed;
- UI coverage proving unavailable post-checkout repository truth removes the
  old repository, Scene/Model tree, and placement presentation;
- generation-aware stale branch completion and active/inactive activation
  coverage retained from the prior M19 checkpoints.

## Runtime and ownership boundary

The backend application service owns `ProjectId` registry lookup, private
repository resolution, manifest identity validation, branch mutation, and
authoritative Project/repository/tree projection. The canonical Tauri host is
the only host wiring for the machine-local registry.

The UI reaches that boundary through `ProjectsGateway` and
`ProjectWriteGateway`; components do not invoke Tauri, Git, filesystem,
OpenUSD, or renderer-cache APIs directly. Runtime construction uses an empty
read model plus `TauriProjectsGateway`; it never falls back to Phase-1 fixture
data. Fixture catalogues and fixture constructors remain test-only.

## Public Project API inventory

The adapter-neutral `project_protocol` surface includes:

- Project list, Project tree, and repository-summary reads;
- location inspection, Project creation/import, root/nested Scene creation,
  Scene adoption, Model import, and branch-switch writes;
- typed Project/Scene/Model/member identities and typed read/write errors;
- `ProjectBranchSwitchRequest`/`ProjectBranchSwitchResponse`, including
  boxed `BranchProjectInvalid` repository truth;
- Scene inspection, Model preparation, import progress, and Project-stage
  activation contracts.

No DTO exposes a filesystem path, Git handle, OpenUSD Stage, renderer object,
or renderer cache key.

## Queue, complexity, and allocation audit

| Boundary | Capacity / policy | Owner |
| --- | --- | --- |
| Scene inspection | one worker plus one replaceable pending job | Project service |
| Model preparation | synchronous capacity `4`, bounded prepared-artifact retention | Project service |
| Stage mutation outbox | capacity `128`, typed `Busy` on saturation | Project service / existing LiveStage owner |
| Import progress | coalesced retention `64` by operation and generation | Project service |
| Project activation preparation | bounded request/result channels of capacity `2` | existing render-host worker |
| Project-stage activation per UI/session | one unresolved activation | existing viewport `SessionCoordinator` |

Registry and manifest identity lookups remain O(1) average through `HashMap`
indexes. Catalogue projection is O(P), tree projection and uncached reusable
Scene search are O(V + E), visible UI rows are O(K), and atomic publication is
O(files changed) plus validation. Branch enumeration is O(B) in the number of
local branches, with repository status delegated to the Git adapter.

M19 does not add a hot-path queue or duplicate Stage authority. IDs and
operation identities are captured instead of whole records; immutable
snapshots are shared; fresh transport DTOs are owned at the boundary; and
invalid-target failure drops stale tree presentation without introducing a
second cache.

## Source-layout audit

The backend source audit passes with **584 files scanned, 44 warnings in the
351–400 range, and 0 failures above 400**. M19 backend files remain within the
hard limit; the largest is `src/project/service/mod.rs` at 399 lines.

The UI M19 diff contains 15 Rust files. New and responsibility-specific
Projects files remain within the 400-line hard limit. Pre-existing legacy
files receiving narrow routing edits remain oversized and are explicitly
recorded rather than hidden: `features/projects/gateway.rs` (680 lines),
`platform/project_host.rs` (601 lines), and `src-tauri/src/lib.rs` (582
lines). No broad refactor or unrelated source-layout cleanup was introduced.

## Gate evidence and limitations

- Backend and UI `cargo fmt --all -- --check`: passed.
- Backend `cargo check --workspace --no-default-features`: passed.
- UI `cargo check --workspace`: passed.
- Backend `cargo test --workspace --no-default-features`: passed; 361 unit
  tests passed, 4 were ignored, and integration suites/doctests passed.
- UI `cargo test --workspace`: passed; 298 desktop tests passed, 1 was
  ignored, and 11 viewport-client tests passed.
- Focused repair tests passed: `usd_git` 5, Project service 4, UI branch
  controller 5.
- `cargo clippy -p project_protocol --all-targets -- -D warnings`: passed;
  the prior `ProjectListItem` large-enum diagnostic is resolved.
- Backend source-size audit: 584 files, 44 warnings, 0 failures.
- Backend/UI `git diff --check`: passed.
- `make harden`: format, source-size, no-default check, and no-default test
  stages passed. The all-features stage remains blocked only by inherited
  environment/dependency conditions: `DLSS_SDK` is unset and the installed
  `wgpu-hal` lacks the Vulkan symbols expected by Bevy.
- Native Tauri host compilation remains dependency-resolution blocked by the
  existing yanked `bisync 0.3.0`/`0.3.1` chain through
  `gix-protocol -> gix -> usd_git`; this is not a M19 source failure.

These are source, compilation, and automated-regression results. They do not
claim live browser/Tauri, GPU, renderer-hardware, or production-deployment
proof.

## Known debt and non-goals

Inherited DLSS/Vulkan hardening and yanked-bisync resolution limitations remain
accepted debt. Commit Graph, Issues/BCF, Team, Turso, renderer-cache ownership,
and new Frost/Glacial Project branches are outside M19. Frost and Glacial were
not changed for Projects Phase 2: **NO CHANGE**.

M19 is the Phase-2 freeze/evidence milestone. No M20 work is present in either
branch.
