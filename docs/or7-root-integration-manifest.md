# OR7-C0 Root Integration Manifest

Status: C0 baseline documented; no root merge performed.

Date: 2026-09-02

Milestone: Owner Review 7 — PeerView root forward-integration

This manifest pins the reproducible OR7 source baseline and records the
semantic conflict classifications required before any root integration. The
root architecture is the receiving architecture. Reviewed history remains
immutable.

## Pinned source baseline

| Repository | Projects source branch | Projects source SHA | Root source ref | Root source SHA | Merge base |
| --- | --- | --- | --- | --- | --- |
| `bevy_openusd` | `develop/project-peerView` | `8eb3c521b17e5ada5d230c229f1548434a6e84ba` | `origin/server/develop` | `e1a0b3d585fc82c70208b391e8c97db18470d700` | `68d3deb4645b030a653f5b27beb514c68073180e` |
| `UsdHubUI` | `projects-peerView` | `1720f31490dc26191d8047ddf9cf27c8f1c37fb3` | `origin/develop` | `be4c603420a28013f2a87a79ea738fa8056fd443` | `d262e53735bae19f4233c11bf531a7286d21d5c0` |

The root refs were fetched at C0 and are the only root commits permitted for
C1 and C4. Later movement of either root branch must not change these pins.

OR7 source variables:

```text
OR7_BACKEND_PROJECTS_SOURCE_SHA=8eb3c521b17e5ada5d230c229f1548434a6e84ba
OR7_BACKEND_ROOT_SHA=e1a0b3d585fc82c70208b391e8c97db18470d700
OR7_BACKEND_MERGE_BASE=68d3deb4645b030a653f5b27beb514c68073180e
OR7_UI_PROJECTS_SOURCE_SHA=1720f31490dc26191d8047ddf9cf27c8f1c37fb3
OR7_UI_ROOT_SHA=be4c603420a28013f2a87a79ea738fa8056fd443
OR7_UI_MERGE_BASE=d262e53735bae19f4233c11bf531a7286d21d5c0
```

## Repository state at C0

- `bevy_openusd`: branch `develop/project-peerView`, local HEAD and
  `origin/develop/project-peerView` both equal
  `8eb3c521b17e5ada5d230c229f1548434a6e84ba`; worktree clean.
- `UsdHubUI`: branch `projects-peerView`, local HEAD and
  `origin/projects-peerView` both equal
  `1720f31490dc26191d8047ddf9cf27c8f1c37fb3`.
- Pre-existing UI change, explicitly outside OR7 scope and not to be staged or
  discarded: `src-tauri/Cargo.lock` adds `thiserror 2.0.20`.
- `bevy_frost` and `bevy_glacial`: audit only at C0; no Project-derived branch
  or library modification is authorized by this checkpoint.
- OR6 is accepted/frozen in the authoritative Projects plan before C0.

## Baseline gates before root movement

### Backend

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS, exit 0 |
| `cargo check --workspace --no-default-features` | PASS, exit 0 |
| `make check-source-size` | PASS, exit 0; existing 351–400-line warnings, no >400-line failure |
| `cargo test --workspace --no-default-features` | BASELINE FAIL, exit 101; 248 passed, 2 failed |

Inherited backend failures:

1. `project::service::m19_tests::phase2_freeze_matrix_covers_create_import_composition_and_recovery` — `DirectoryNotEmpty` during disposable fixture cleanup at `src/project/service/m19_tests.rs:103`.
2. `project::service::stage_mutation::tests::deleting_an_inactive_scene_is_consumed_without_mutating_the_active_stage` — inherited assertion expecting the pre-existing canonical stage shape without `SceneRoot` at `src/project/service/stage_mutation_tests.rs:226`.

### Frontend

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS, exit 0 |
| `cargo check --workspace` | PASS, exit 0 |
| `cargo test --workspace` | BASELINE FAIL, exit 101; 334 passed, 1 failed, 1 ignored |

Inherited frontend failure:

- `components::model_view::built_ins::registry::tests::built_ins_register_with_stable_ids_and_render_through_the_host_contract` — `reactive_graph` attempted `spawn_local()` before a global executor was initialized.

These failures are baseline evidence. They are not OR7 integration failures
unless their behavior changes after the relevant checkpoint.

## Semantic conflict manifest

Each listed high-risk path must be resolved according to its classification,
not by blindly choosing one side of the merge.

### Backend paths

| Path | Classification | Root behavior that must survive | Projects behavior that must survive | Expected verification |
| --- | --- | --- | --- | --- |
| `Cargo.toml` | MANUAL ADAPT | Root workspace/features/dependency architecture | Project-required crates/features | workspace checks and feature resolution |
| `Cargo.lock` | MANUAL ADAPT | Root lock resolution | Accepted Project lock requirements | lock consistency and workspace checks |
| `crates/usd_bevy/src/lib.rs` | ROOT KEEP | Root module/API assembly | Only still-valid Project exports | usd_bevy compile/tests |
| `crates/usd_bevy/src/live/*` | ROOT KEEP | Runtime isolation, projection lifecycle, animation path | Project activation hook where compatible | live/reconcile/animation tests |
| `crates/usd_bevy/src/live/reconcile/*` | ROOT KEEP | `PathStore`, `ChangePlan`, sparse reconciliation | Narrow Project hierarchy revision notification | reconcile/path/dependency tests |
| `crates/usd_bevy/src/live/stage.rs` | ROOT KEEP | authoritative Stage and batch ownership | Project presentation/invalidation hook | stage lifecycle tests |
| `crates/usd_bevy/src/route/geom.rs` | MANUAL ADAPT | dense hierarchy/geometry routing and compact IDs | Project semantic labels/metadata opinions | hierarchy and geometry tests |
| `crates/usd_bevy/src/route/material/*` | ROOT KEEP | material cache and bounded material routing | No Project-specific regression | material/cache tests |
| `crates/usd_bevy/src/route/mod.rs` | ROOT KEEP | root route module boundaries | compatible Project route registration only | route compile/tests |
| `crates/usd_git/src/repository.rs` | ROOT KEEP | root Git/repository ownership and path identity | Project storage behavior through existing APIs | repository and Project storage tests |
| `crates/usd_semantic/src/config.rs` | BIM-SPECIFIC ROOT KEEP | root semantic/BIM provider configuration | Project activation context through typed inputs | semantic/BIM configuration tests |
| `src/project/ghost_cache/*` | MANUAL ADAPT | derived/rebuildable cache and bounded payload behavior | accepted Project cache/activation semantics | cache and activation tests |
| `src/project/runtime_delivery.rs` | MANUAL ADAPT | bounded runtime delivery and ownership | Project activation/runtime delivery contract | delivery and activation tests |
| `src/viewport/bridge/*` | MANUAL ADAPT | root gateway/protocol boundary | Project typed intents and host routing | protocol/bridge tests |
| `src/viewport/api/hierarchy.rs` | MANUAL ADAPT | root hierarchy read-model/source protocol | `StagePresentationContext`, target naming | hierarchy protocol tests |
| `src/viewport/api/scene_index*` | MANUAL ADAPT | dense Scene/Hierarchy index and O(1) lookup | Project metadata revision/invalidation hook | scene-index/hierarchy tests |
| `src/viewport/app/runner.rs` | MANUAL ADAPT | root app/runtime scheduling and working set | Project activation wiring without hot-path scans | app/runtime tests |
| `src/viewport/semantic_sync/*` | ROOT KEEP | immutable semantic snapshot sync and bounded workers | Project provider/generation identity | semantic sync/BIM tests |
| `src/viewport/session/lifecycle.rs` | MANUAL ADAPT | root session lifecycle and generation safety | Project activation/cache context | lifecycle and stale-state tests |

### Frontend paths

| Path | Classification | Root behavior that must survive | Projects behavior that must survive | Expected verification |
| --- | --- | --- | --- | --- |
| `apps/desktop/src/components/model_view/built_ins/registry.rs` | ROOT KEEP | Panel registry/host contract and root BIMData registration | accepted Project built-ins only | registry/panel tests |
| `apps/desktop/src/components/model_view/built_ins/scene_tree.rs` | MANUAL ADAPT | root hierarchy dataflow and virtualized host | Project contextual naming/navigation | hierarchy/UI tests |
| `apps/desktop/src/components/model_view/built_ins/scene_tree/rows.rs` | ROOT KEEP — deletion protected | root removed this obsolete implementation | no resurrection of old per-row logic | tree module compile and source audit |
| `apps/desktop/src/components/model_view/model_view.rs` | MANUAL ADAPT | root model-view shell and viewport integration | Project PeerView layout/navigation | model-view tests |
| `apps/desktop/src/components/model_view/panels/context.rs` | MANUAL ADAPT | root panel context/source identity | Project contextual panel semantics | panel/context tests |
| `apps/desktop/src/features/viewport/command_gateway.rs` | ROOT KEEP | typed command gateway and sequencing | Project intents routed through host | gateway tests |
| `apps/desktop/src/features/viewport/controller.rs` | MANUAL ADAPT | root controller/reducer ownership | Project activation intent wiring | controller/activation tests |
| `apps/desktop/src/features/viewport/event_reducer.rs` | ROOT KEEP | authoritative event reduction and stale filtering | Project event adaptation only | reducer/protocol tests |
| `apps/desktop/src/features/viewport/mod.rs` | ROOT KEEP | root viewport module split | Project integration exports without duplication | frontend compile/tests |
| `apps/desktop/src/features/viewport/remote_session.rs` | ROOT KEEP | remote session lifecycle and bounded transport | Project activation remains typed | session tests |
| `apps/desktop/src/features/viewport/scene_cache.rs` | ROOT KEEP | generation/source-keyed cache and paging | Project presentation context carried by read model | scene-cache stale-state tests |
| `apps/desktop/src/features/viewport/store.rs` | MANUAL ADAPT | root store/query split and immutable read models | Project PeerView state/actions | store/activation tests |
| `apps/desktop/src/features/viewport/store/b9_tests.rs` | ROOT KEEP | root acceptance regressions | no Project behavior may remove them | store acceptance tests |
| `apps/desktop/src/features/viewport/store/internal.rs` | ROOT KEEP | internal root state ownership | compatible Project projection only | store tests |
| `apps/desktop/src/features/viewport/store/queries.rs` | ROOT KEEP | bounded read-model queries | Project labels from authoritative models | query tests |
| `apps/desktop/src/features/viewport/store/remote.rs` | ROOT KEEP | typed remote/session state | Project activation routing | remote session tests |
| `apps/desktop/src/features/viewport/store/scene.rs` | ROOT KEEP | scene paging/cache mutation boundaries | Project selection intent integration | scene-cache tests |
| `style/tailwind.css` | MANUAL ADAPT | root retained viewport/panel layout constraints | accepted Projects presentation/layout | frontend build and visual code review |

## C0 acceptance and next checkpoint

- OR6 accepted/frozen before source integration.
- Exact backend and frontend Projects source SHAs recorded.
- Exact root SHAs fetched and pinned.
- Merge bases recorded.
- No root merge performed in C0.
- All required high-risk paths have a semantic classification.
- Baseline failures are recorded before source changes.
- Root performance preservation requirements are explicit.
- UI pre-existing `Cargo.lock` work is preserved and excluded from OR7.

C0 commit:

```text
Owner review 7-C0: pin root integration baselines
```

The next authorized action is C1: merge only the pinned backend root SHA with
`git merge --no-commit --no-ff`, resolve classified overlaps semantically, run
the C1 gates, commit, push, and continue to C2. Do not update the pins or
begin C4 frontend movement from a newer root branch.
