# UP-C07 I5 — Code/documentation closure and qualification handoff

> **Scope:** Mission `M-NEM-MIG-20261119-PC-CDS16-01`, plan revision
> `sha256:45f10c1cba193275f2a7d617f9a1aaaeb9d4a9555855343881935ea11065fbc6`,
> checkpoint UP-C07, Iteration 5.
>
> **Source basis:** `bevy_openusd` HEAD `1aeca866d2271ee6adb0d54f069a112cee7f33e8`;
> `UsdHubUI` HEAD `6f5f747d30fc6b8d896859744c6c546401f4fa62`;
> reviewed *uncommitted paired* source fingerprint
> `92e67ddfa3346fedb247adb47f42bbaadd515b4dff2d62d3f2bba8aa1a8c4c8c`.
>
> **This patch's purpose:** Put the complete, implementation-ready **code/documentation**
> disposition of every UP-C07 I5 checklist item in the repository. This is a
> **new documentation file only**, not a code change, execution receipt, new
> Reviewer verdict, acceptance, or a waiver of missing qualification. It does
> not change the Implementer-authored authoritative working plan.

## 1. Definitive disposition: do not invent a source fix

The independent I5 review identifies **no new confirmed CODE FAIL**. The
file-backed discovery hardening is already present and passed its registered
focused tests; the configured Compile was 8/8 PASS and Local Agent formal Test
was 16/16 PASS **on the exact I5 fingerprint above**. The relevant focus logic
is not a candidate for speculative replacement. The Owner has not waived any
required connected/sustained qualification. Thus:

- **Code-side correction still required by this checklist:** **NONE CONFIRMED**.
- **Documentation/code-review handoff:** supplied by this file; status
  `DOCUMENTED — NOT A RUNTIME PASS`.
- **E-C07-04 and E-C07-I5-COLD:** `NOT_VERIFIED` until new connected receipts.
- **R-C07-I5-01:** `NOT_VERIFIED` occurrence-specific native/projection race;
  no source defect can be asserted until the actual race is reproduced.
- **P-C07-I5-01:** Owner-selected GPT-6.1 Sol High / effort profile issue is
  procedural, **not** a product-code defect; a repository patch cannot cure it.
- **Overall Reviewer result:** `BLOCKED` (Phase 1 `BLOCKED`, Phase 2
  `APPROVED` only for unconfigured separate rule documents). That result is
  immutable; no doc commit changes it.

### 1.1 Verified source contracts to preserve

| Area | Existing code / symbol | Required invariant / regression oracle | I5 scope status |
|---|---|---|---|
| File-backed/documentless detection | `crates/usd_bevy/src/live/stage.rs::LiveStage::visual_loadable_paths_if_backed` | `Ok(None)` **only** when visual Stage is absent; `Ok(Some(paths))` for discovered file-backed owners; operational synchronization/discovery error is `Err` | CODE PASS, focused |
| Focus error propagation | `src/viewport/api/bridge/focus_bounds.rs::focus_params_for_payload` | Map `Err` to `"cannot verify visual payload owners for focus"`; do **not** silently fit currently resident entity when file-backed source coverage is unknown | CODE PASS, focused |
| Correlated rejection | `src/viewport/api/bridge/tree.rs::apply_tree_commands` | Exactly the original `FocusTarget` request ID on `CommandRejected`; **leave existing `FlyTo` unchanged** on error | CODE PASS, focused |
| Complete ordinary-anchor bounds | `focus_bounds.rs::canonical_fit_params` | Unique ordinary canonical target uses complete source `BBoxCache`, extents hint disabled; Stage up-axis once; no visual/canonical source writes | CODE PASS, focused + paired |
| Disconnected/occurrence fallback | `focus_params_for_payload` and `SceneAnchorIndex` identity route | Documentless/nonoverlap continues prior entity fit; occurrence-specific world transform must not be replaced by unqualified canonical prim bounds | CODE PASS in focused test, *transient race* NOT_VERIFIED |
| Native visual and canonical separation | `LiveStage` visual load/readback APIs; `visual_composition.rs` | Native visual load batches and session opinions never reduce authoritative QueryPrim/export; preserve manual owner rules | CODE PASS/partial connected |
| Authored LOD | `src/viewport/residency/visual_lod.rs` | Session-only changes, exact selected fallback, no persistent authored variant mutation | CODE PASS/partial connected, full corpus NOT_VERIFIED |
| Generated LOD | `src/project/cache/derived_lod.rs`, `src/viewport/residency/generated_lod.rs` | Opt-in, deterministic source-mapped candidates; preserve original geometry/UV/material/safe rejection and exact query | CODE PASS/partial connected, full corpus NOT_VERIFIED |
| Budget and hierarchy | `src/viewport/residency/authority/capacity.rs`, `crates/usd_bevy/src/live/{system,progressive_resident}.rs` | One residency authority, bounded transitions, current generation and anchors retained, no unintended whole-Stage restart | CODE PASS/partial connected, large-scale NOT_VERIFIED |

The exact I5 regression `tests/unit/viewport/api/bridge/tests/selection/selection_performance_tests.rs::focus_keeps_canonical_nested_bounds_after_visual_unload`
opens a real file-backed Z-up USD, unloads its nested payload, asserts the
canonical Y-up fit `(0,6,0)`/distance `22.4`, authors a missing-file reference,
observes native discovery error, then verifies a request-correlated rejection
and unchanged `FlyTo`. The neighboring
`focus_uses_resident_geometry_for_occurrences_and_disconnected_targets`
checks healthy documentless and disconnected fits. These tests are necessary,
**not substitutes** for connected batch failure, large-scene stress, or the
occurrence projection gap. Do not duplicate or weaken them.

### 1.2 Evidence already available; exact limitations

- I5 paired Chromium/WebRTC active/disabled on the same SHA-256-identified
  executable and source fixture: six Project activations each, 30 focus
  transitions per mode, 61 active native payload batches, ten authored LOD
  actions, 34 generated admissions, and decoded Far/Section Box images. Five
  matched canonical export hashes prove those **specific** exact outputs.
- In the **small fixture only**, the late 12 samples in a ~56-second run show
  WDDM dedicated VRAM flat at 792,834,048 B in both modes. This is not a
  larger federation queue/RSS/VRAM plateau or teardown proof.
- Fresh-process/warm-cache probes were analyzed **after explicit Near focus**.
  The active frame at +384 ms from focus is a **cropped transition**; the
  disabled frame at +370 ms is recognizable Near geometry. Neither qualifies
  **automatic, pre-focus first useful displayed frame**. An axes-only
  detector was specifically rejected as a false positive.
- The file-backed missing-reference negative test exercises native discovery,
  **not** a connected `apply_visual_load_batch` or `load_set()` error/recovery.
- The native loaded-set/Bevy projected-entity **occurrence-specific** gap has
  no connected result. Do not infer a bug or a PASS.

## 2. Outstanding checklist: exact acceptance contracts

Every row below is **NOT_VERIFIED** until the **Local Agent** produces the
specified output from a source-matched run with the Owner-approved oracle.
This file only documents what to collect; it does not create evidence.

| ID / scope | Required setup and observable proof | Pass oracle | Fail / stop outcome |
|---|---|---|---|
| `E-C07-I5-COLD` / S01 | Fresh server+browser for both *cold cache* and warm Partial cache; no `FocusTarget` before the measurement. Preserve every actually **presented/decoded** initial frame, Stage/revision/generation, expected named-prim silhouette, camera pose, initial activation ACK and frame timestamp. Repeat active/disabled; native host if Owner requires. | First *automatic* complete named-target frame is identified with calibrated latency and no cropped target or axes-only false match | If automatic target is missing or permanently clipped after allowed settle, preserve RED trace and request precise camera/first-display source owner; do not extrapolate from post-focus transition |
| `R-C07-I5-01` / S01 | File-backed Stage with two occurrences of one prim at **distinct world transforms**. Force native unload/reload and focus one occurrence **between native loaded-set readback and Bevy entity readiness**. Record occurrence ID, canonical/reference/world bounds, projected entity AABB, focus ACK/reject, camera pose and displayed frame, then alternate to other occurrence and repeat. | Correct *occurrence-specific* fit or clearly correlated safe non-success; never false successful fit at wrong instance; no stale delayed camera movement | A wrong center or crop with a success ACK is a new verified CODE finding. Freeze RED and obtain actual command/projection code before patching |
| `E-C07-04` / S01 failure | Under normal connected product host, separately cause a **real native batch apply failure** and a **real native loaded-set readback failure** during active visual unload/reload. Capture error, `StageLoadState` Loading/Failed, retry/backoff, maximum batch, manual payload rules, authoritative queries, native loaded set and eventual recovery. | No false `Ready`, no permanent quarantine/stale source, retry follows existing bounded parent-first policy, full readback or explicit failure | A missing-file discovery rejection by itself does not satisfy this row; preserve distinct failure injection and stage-readback timestamps |
| `E-C07-04` / S02+S03 full fidelity | Authored 0/1/2 and missing/invalid/nested variants; opt-in generated static candidates with UV seams, materials, holes, normals, negative/unsupported cases; each in normal, selected and Section Box modes. Record aligned images, flicker, exact pick/measurement/query, source digest and rejected-candidate fallback. | Engineering interactions use exact source geometry; no accidental persisted variant, broken anchor, unsafe generated candidate, clipping or nonapproved image error | Source change only after a specific reproducible geometry/variant/picking mismatch |
| `E-C07-04` / S04 Project Git | With a nested visual owner natively unloaded, run semantic/engineering QueryPrim, edit, Undo/Redo, save, Project Git commit/branch/history replay/checkout and full export. Compare active/disabled source closure and effective values at every checkpoint. | Source-complete queries and reproducible Project Git history; exact current authored state independent of optional visual residency | A mere pair of equal exports **without** commit/replay does not close this row |
| `E-C07-04` / S04 sustained federation | Owner-defined realistic multi-source federation, controlled CPU/GPU budgets and duration; camera sweeps, selected/mounted, hide/show, LOD churn, edit/revision, failures and teardown. Full timestamped RSS, private bytes, **per-process WDDM** VRAM, actual queue depths, per-cycle peaks/settled baselines, resident/warm/pinned/evicted counts, frame latency. | Owner-calibrated bounded queue and resource plateau without monotonic growth hidden by averages, and clean releases | A ~56-second synthetic scene, final-window-only or device-wide nvidia-smi number does not close this row |
| C07 adjacent native/remote host | Same source, camera, selection and Section Box across native GUI and Chromium/WebRTC, including resize and supported render modes; preserve selected anchor and frame results | No host-specific false completeness, wrong camera mount, or missing semantic result | Qualify only the exercised clients; disclose unsupported configurations |

**Preconditions for ALL gates:** exact tested source fingerprint, executable
SHA-256, fixture source digests, Stage identity/authoring revision, cache
Partial/Ready generation and visual policy settings; immutable source-bound
oracles; exact command/order and client timestamp; Owner-approved quantitative
thresholds **before** the run. Never report PASS based solely on `cargo test`,
formal Test 16/16, a single final image, or the working plan's assertion.

## 3. Implementation-ready diagnostic sequences (not executed)

### 3.1 Cold first useful frame — avoid another false-positive detector

1. Obtain from Owner the expected startup behavior: automatic full-scene
   initial view and which named prim must be visible without a user focus.
   Record target/world bounds and a **complete silhouette** in the fixture,
   not generic grid, world axes, gizmos or a grid-only screenshot.
2. Freeze binaries/source/scene and disable **all** user-issued focus until
   the cold observation interval ends. Use four combinations:
   active/disabled × warm Partial cache/cold cache. Keep native GUI as a
   separately identified host when required by the Owner.
3. Capture stage-open, activation ACK, native load set, projection-ready,
   encoder output and **browser-presented** frames with a shared timeline.
4. Reject mismatched Stage generation, axes-only candidate, incomplete
   silhouette, clipped/cropped transition and a frame created only after a
   FocusTarget. Do **not** substitute server `projection_complete` time for
   actually displayed useful pixels.
5. Record earliest *qualifying* frame, activation-to-display latency and
   matching active/disabled frame. Repeat at least the Owner-agreed run count.
6. Only **after** the autonomous measurement, issue explicit FocusTarget and
   separately classify transient transition frames. If target never appears,
   stop; do not add implicit autofocus or a second camera scheduler without a
   precise RED trace and Owner-approved intended startup contract.

### 3.2 Occurrence during native/projection mismatch

1. Create a file-backed duplicate/prototype fixture with two occurrences
   carrying different instance IDs and different world transforms. Precompute
   the exact expected center/radius **per occurrence**, not the shared prim.
2. Record native unload and `load_set()` readback for one occurrence, then
   native reload without yet completing Bevy entity/anchor reconcile. Verify
   from the actual trace that the requested race interval existed.
3. Send one `FocusTarget` with the full occurrence context through the
   existing bridge; record its request ID, result event, old/new `FlyTo`,
   source revision and selected anchor. Observe a visible frame after settle.
4. Repeat with the other occurrence, rapid alternating focus, manual orbit,
   Stage replacement, invalid payload and disabled control. Preserve
   cancellation semantics: older work must not override a newer user intent.
5. If correct or safely rejected, mark the specific case qualified. If
   ACK-success uses the *wrong* occurrence center or clips the current
   occurrence, freeze evidence and inspect **then** the precise verified
   bridge/SceneAnchorIndex/projection owner for the smallest possible fix.
   Do not preemptively substitute shared canonical bounds for occurrences.

### 3.3 Real native apply/readback failure vs discovery failure

1. Distinguish the already-tested `visual_loadable_paths_if_backed()`
   composition/discovery error from **native batch** error and **post-apply
   loaded-set readback** error. They are different execution points.
2. Through a Local-Agent-approved connected fault seam, force each failure
   separately on an otherwise healthy file-backed Stage; keep the canonical
   source unchanged. Record every failed/successful batch and its owner list.
3. Assert bounded parent-first admission (existing 8-per-attempt limit where
   applicable), backoff, StageLoadState Loading/Failed and eventual native
   loaded-set proof **before** any Ready-success statement. Keep manual-load
   choices, edit pins and logical hierarchy; no permanent failed-owner
   quarantine or false canonical source change.
4. On repeated failure show explicit non-success, no busy loop and no partial
   geometry presented as complete. On recovery verify exact query/export,
   selection and no whole-stage progressive restart caused solely by the
   visual-only change.
5. If an invariant fails, freeze the *connected* RED receipt and only then
   scope a source fix in `LiveStage::apply_visual_load_batch`,
   `visual_composition.rs::restore_visual_owners` or the **actual** error
   owner verified in that trace; do not guess at an unavailable API.

### 3.4 LOD/precision and Project Git

- Authored: include nested/invalid missing variants; compare source default,
  selection/section exact geometry, image silhouette/flicker and preserved
  layer/variant authoring. Never treat visual session opinion as a Git edit.
- Generated: opt-in only; test material, UV, seam, hole, winding and
  unsupported skinning/animation; record chosen simplifier recipe/source
  digests and rejection fallback. Picking/measurement/query must still access
  exact full geometry, not the derived candidate.
- History: while owner unloaded, query/author/Undo/Redo, commit, branch,
  checkout/history replay and export active and disabled modes. Compare full
  authoritative source closure. Distinguish effective Undo value from exact
  source layer byte history, because inherited-value authoring can differ.

### 3.5 Resource/queue qualification

- Agree on large-scene and soak-duration thresholds **before** execution;
  include normal and streaming-disabled control and CPU/GPU settings.
- Track timestamped **per-cycle peaks** and warm settled baselines of RSS,
  private bytes, WDDM process-dedicated GPU, queue depth and frame latency,
  plus logical, pinned, warm/evicted and generated resource counts.
- Capture stage replacements, camera sweeps, edit churn, cache invalidate and
  repair, unload/reload and teardown. Do not average away growing peaks.
- Correlate preexisting invalid-entity despawn warnings with actual wrong
  output/identity before promoting them to a code finding; warnings by
  themselves do not prove a product defect.

## 4. Conditional source change decision tree — NOT a patch instruction

```text
for each missing gate:
    Local Agent checks exact build, fixture, oracle and complete observation
    if observation missing/ambiguous:
        status = NOT_VERIFIED
        request the precisely identified missing evidence
    else if an observable invariant is violated:
        status = FAIL
        preserve reproducible RED trace, source and binary identities
        identify exact actual production source owner and its contracts
        Owner approves a narrowly scoped correction in a NEW iteration
    else:
        status = PASS for that gate + exact build/fixture/platform ONLY

# Never convert an evidence gap into a fabricated product bug.
# Never treat documentation, Compile or formal Test as a connected PASS.
# A later Owner acceptance does not revise an immutable Reviewer verdict.
```

**PROPOSED / NOT EXECUTED.** This is an evidence/decision protocol, not Rust
source code. In a new authorized source patch, keep the existing `LiveStage`,
focus bridge, session LOD and residency authority as the single respective
owners. Do not add simulation/perception services, alter the camera protocol
or canonical USD layers, duplicate an engine scheduler or introduce an
additional Project cache authority to paper over a missing measurement.

## 5. Ownership, handoff and exact completion checklist

1. **Implementer** inspects the *actual* uncommitted I5 working tree, verifies
   both baseline HEADs and the I5 fingerprint, and imports this **new doc
   only**. If the destination document already exists, stop and reconcile;
   do not overwrite a different document or reset the worktree. The
   Implementer, not Reviewer, may update `authoritaritative_plan.md` with
   factual import/status and later executed outcomes.
2. **Implementer Compile** uses the *configured commands* in the I5
   `compile.log` (backend workspace/Solari/example/architecture, frontend
   workspace/native/install/build) on any future **changed source**. The I5
   8/8 receipt remains historical for the I5 fingerprint, not a PASS for a
   new fingerprint.
3. **Local Agent formal Test** owns the configured 16 suites and connected
   experiment matrix, including file-backed discovery, correlated rejection,
   preservation of FlyTo, documentless fit, canonical source identity and
   the full E-C07-04 gates. The I5 Test PASS 16/16 is historical on I5.
4. **Owner** approves workload and thresholds, authorizes any extra
   sources/fixtures for a new frozen reviewer packet, and resolves the
   requested Reviewer UI-selected model/effort profile discrepancy. No repo
   doc can verify product UI controls.
5. **A future Reviewer** independently assesses the new fingerprint and
   exact allowlisted receipts. Open gates remain NOT_VERIFIED until then;
   the I5 `BLOCKED` verdict is immutable.

### Do-not-close register

- [ ] `E-C07-I5-COLD`: first automatic useful **displayed** frame observed,
      before any FocusTarget, on approved cold/warm fixture matrix.
- [ ] `R-C07-I5-01`: connected per-occurrence native-load/projection gap
      resolved as correct focus or safe correlated non-success.
- [ ] `E-C07-04-S01`: connected actual batch apply **and** loaded-set readback
      failure with bounded retry and no false Ready.
- [ ] `E-C07-04-S02/S03`: selected/Section/LOD full precision and image corpus.
- [ ] `E-C07-04-S04-history`: full Project Git branch/commit/replay/export
      and complete exact semantic query while payload unloaded.
- [ ] `E-C07-04-S04-resource`: Owner-approved large-federation sustained
      queue, RSS/private/VRAM and teardown plateau.
- [ ] Native/remote client parity and no stale projection as scoped by Owner.
- [ ] Reviewer-profile authorization addressed by Owner's actual UI controls.

**Code/documentation-only disposition:** All source correction opportunities
that are *actually proved* by I5 are already addressed. This new document
closes the code/documentation **handoff**, not any of the above missing
execution/evidence gates. Do not mark those checkboxes until the Local Agent
obtains the corresponding receipts and a new authorized review determines
status. No fake screenshots, tests, benchmark values, source commits or
retroactive PASS statuses are introduced by this patch.
