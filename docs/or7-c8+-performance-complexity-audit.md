# OR7-C8+ integrated performance complexity audit

Status: complete for the corrected OR7 implementation. This is a source and
ownership audit, not a runtime benchmark. The audit records the input size,
the reviewed pre-OR7 shape, the final shape, allocations, invalidation, and
execution ownership for every materially modified backend path.

## Notation and evidence boundary

`B` is the number of StageChangeBatch entries, `Q` the number of changed-info
paths, `R` the number of minimized resync roots, `S` the number of prims in
the affected subtrees, `N` the number of projected stage prims, `M` the number
of semantic entities, `H` the number of hierarchy nodes, `D` the number of
dependency consumers, and `P` the bounded page size. A full fallback is
explicitly O(N) or O(M); it is not reported as sparse work.

The complexity statements below are asymptotic source inspection. No FPS,
latency, memory, throughput, Revit, Omniverse, GPU, native-window, or
production claim is inferred from compilation or unit tests.

## Path audit

| Material path | Pre-OR7 / rejected shape | Final corrected complexity | Allocations and repeated work | Invalidation, owner, and execution |
| --- | --- | --- | --- | --- |
| `crates/usd_bevy/src/live/change.rs` -> `StageChangeBatch` and `crates/usd_bevy/src/live/system.rs` -> drain/fan-out | Independent consumers could drain or rebuild from raw notices; path strings could be rematerialized at each consumer. | One drain is O(B + Q); `resync_roots` is bounded by the notice set and root minimization; later consumers share the retained revision. | The authoritative `Vec<StageChange>` is copied once out of the OpenUSD callback. No second model is created for each consumer. Root and changed-info helper vectors are batch-local. | Replaced once per frame by `drain_stage_changes_system`; Bevy owner thread; `PendingStageChanges` is a transient fan-out resource, not identity authority or an obsolete resync API. |
| `crates/usd_bevy/src/live/reconcile/mod.rs` -> `ChangePlan` and sparse patching | String-owned per-notice planning and repeated per-property dependency lookup were the rejected risk. | Plan construction is O(Q) expected hash work plus property de-duplication; distinct prim patching is O(U + D), where U is distinct owning prims. Resync reconciliation is O(S) for subtree work and O(N) only for an explicit full fallback. | `PathStore` interns canonical paths once; `ChangePlan` stores `PathId` and borrowed property names. One `HashMap`/vector plan and dependency fan-out are allocated per batch. No full-stage walk occurs for ordinary changed-info. | Invalidated by a new batch. `PrimEntities`, `PathStore`, and dependency indexes remain viewport-owned; the Bevy owner thread applies patches. `StageChangeBatch` remains the receiving architecture. |
| `crates/usd_bevy/src/live/reconcile/subtree.rs` and `full.rs` | A root change was previously treated as if every change were the same granularity. | Minimal roots and current subtree collection are O(R + S) apart from path-set/hash and sorting costs; removal/spawn/patch visits are O(S). Root `/`, missing parents, invalid paths, or collection failure deliberately use O(N) full reconciliation. | Subtree reconciliation allocates old/current path sets and sorted path vectors sized to affected work. Full reconciliation allocates the complete current-path set and ordered list. These are explicit fallback allocations, not per-frame idle allocations. | Triggered only by resync roots in the retained batch or a fallback condition. `PrimEntities`, semantic index, material consumers, point-instancer dependencies, and native-instance dependencies are mutated on the Bevy owner thread. |
| `src/viewport/semantic/sync/mod.rs`, `identity.rs`, `subtree.rs`, `changed_info.rs` | The rejected integrated head called the old API on `PendingStageChanges` and did not compile. It therefore had no valid runtime complexity. | Initial/replacement extraction is O(M) over the stage snapshot. Subtree delta work is O(M + S) because the immutable working entity map is rebuilt after extracting affected entities; changed-info delta is O(M + Q) and explicitly traverses available stage paths before extraction. Sparse delta work is selected from the typed `StageChangeBatch`; invalid/empty/root failures use the explicit O(M) fallback. | A previous snapshot is handed to the worker as `Arc`; the delta builder may clone its entity map internally to form the next immutable snapshot. No full `SemanticSnapshot` is cloned merely to cross a thread or mailbox. | New revision, session, activation generation, or a non-empty batch invalidates the prior state. `synchronize_live_stage` runs as an exclusive Bevy system while borrowing non-send `LiveStage`; semantic extraction is owner-thread work, then the immutable action is sent onward. |
| `src/viewport/semantic/worker.rs`, `mailbox.rs`, and `sync/delivery_worker.rs` | Unbounded query/result or delivery growth was the reviewed risk. | Semantic state/query mailboxes keep latest work bounded; query reduction is O(M) for snapshot replacement and query-dependent for indexed reads. Runtime delivery queue depth is fixed; submission/drain work is O(1) per item excluding payload construction. | Snapshots and prepared payloads cross worker boundaries through `Arc`/owned immutable data. The latest mailbox drops superseded pending work; delivery queue coalesces only work not yet started. No unbounded queue is introduced. | Worker threads own Turso/semantic query and runtime blob reads. Sync result mailboxes are bounded and backpressure is recorded. Bevy drains results and applies generation/revision identity on the owner thread. |
| `src/viewport/api/scene_index_rebuild.rs`, `scene_index_dense.rs`, `scene_index_hierarchy.rs` | The primary hierarchy contract could be mistaken for a `HashMap<Option<SceneAnchor>, Vec<_>>` and repeated child scans. | Rebuild is O(H log H) for deterministic ordering plus O(H) dense/index construction. Child page lookup is O(1) to obtain a range plus O(P) projection. Search reveal ancestry is O(depth) after dense parent links are built. | Rebuild allocates dense nodes, path/anchor maps, child ranges/order, and occurrence data once per invalidation. A page allocates only its bounded protocol rows. No per-row ancestry scan occurs during paging. | Rebuilt when projected scene rows, hierarchy metadata, presentation context, or lifecycle markers change. `SceneAnchorIndex` and dense indexes are session-local Bevy resources; no worker owns ECS entities. |
| `src/viewport/api/scene_query.rs` and `scene_query_projection.rs` | Search could block the render schedule or allow stale results to overwrite a newer query. | Each hierarchy search scans its immutable input O(H) plus name/query character work and returns at most the protocol bound. BIM search uses the immutable snapshot/index and is query-dependent; result publication is O(1) mailbox replacement. | One latest-value job and one latest-value result are retained. `Arc<HierarchyReadModel>`, `Arc<SemanticSnapshot>`, and `Arc<BimReadIndex>` avoid full snapshot copies between owner and worker. | A dedicated `usdview-scene-search` worker owns search execution. `activation_generation`, request ID, source, and query are carried back; the owner drops stale results. |
| `src/viewport/bim/index.rs`, `bim/search.rs`, `bim/classification.rs` | Repeated full property scans and one material/BIM object ownership would make reads and colorization scale with avoidable duplication. | BIM index build is O(M + property occurrences) and is invalidated by a new snapshot. Property/name reads use postings; classification build is O(M) per uncached recipe and cached by immutable snapshot/index. | The index owns boxed entity/property postings and bounded recipe cache; values remain in the semantic snapshot. Color groups map occurrences and do not allocate one material per BIM object. | Snapshot ID and recipe change invalidate the relevant cache. `BimReadService` borrows the snapshot; query/search work is worker-owned, while ECS material application remains owner-thread work. |
| `src/viewport/session/lifecycle.rs`, `lifecycle_invalidation.rs`, `src/viewport/app/project_activation.rs` | Project activation could leave late scene/BIM responses attached to the prior stage generation. | Activation bookkeeping is O(1); derived-state reset is proportional to the number of cleared local structures, not a stage-wide search. Preparation and response processing are bounded by their queue capacities. | Previous client/cache handles are retained only for rollback; accepted activation moves a new generation into the current store. Preparation and result channels are bounded. | Activation generation is the invalidation key. Native/OpenUSD preparation is worker-owned; final stage replacement, cache reset, and response publication are Bevy owner-thread operations. |
| `crates/usd_bevy/src/live/native_instance_dependency.rs`, `projection.rs`, `progressive.rs` | Native instance dependencies could be recovered by scanning all consumers or by treating replaceable ECS entities as identity. | Full dependency index rebuild is O(N) after initial/full projection. Scoped refresh is O(ancestor depth + affected consumers); lookup is expected O(1) for the reverse map. | The reverse index owns compact path relationships; no per-instance mesh asset is created for shared prototype geometry. Scoped refresh allocates only affected records. | Full rebuild follows initial/full projection; scoped refresh follows a localized reconcile. `NativeInstanceDependencyIndex` owns dependency identity; `PrimEntities` remains a replaceable ECS mapping. Bevy owner thread updates it. |
| `src/viewport/scene/selection.rs`, `selection_outline.rs`, and render/visibility routes | Selection or cancellation could rely on current entity allocation instead of stable identity and could redo broad work. | Stable anchor/instance identity capture is O(1) per selected entity; outline reconciliation is proportional to changed/desired selections and bounded queue work. | Desired selection sets are owned by the selection route; cancellation does not require rebuilding unrelated scene state. | Invalidated by selection or active-stage generation. Selection and visibility are Bevy owner-thread routes; bounded work queues report saturation rather than growing without limit. |

## Explicit negative checks

Source inspection confirms the corrected OR7 implementation does not restore:

- a string-owned reconcile plan as the primary renderer architecture;
- `PrimEntities` as Project or semantic identity authority;
- a primary hierarchy `HashMap<Option<SceneAnchor>, Vec<_>>` in place of dense
  parent/child ranges;
- per-row ancestry scans or a full-tree frontend clone/re-sort in the backend
  paging/reveal path;
- full `SemanticSnapshot` clone handoffs across worker boundaries;
- unbounded semantic, search, runtime-delivery, or activation workers;
- per-frame filesystem, Git, or OpenUSD traversal in the idle render path; or
- one material/BIM object as the colorization ownership model.

The required C1+ API correction is also preserved: semantic sync receives the
root `StageChangeBatch`, and `PendingStageChanges` is used only as its
same-frame retained batch resource. No obsolete resync API was reintroduced.

## Evidence

- Backend C1+ accepted commit: `4d655c67ce3b3ded3796c254593ca5484d30ab5b`.
- Frontend C5+ commit: `267fe24` (pushed on `projects-peerView`).
- Focused backend and frontend test results are recorded in the C9+ matrix;
  this C8+ record intentionally does not convert those results into runtime
  performance claims.
- `cargo fmt`, workspace compilation, test execution, source-size, and harden
  outcomes are recorded at C10+ after the integrated matrix is run.

No runtime profile was available. Owner Review remains required.
