# UP-C07 Iteration 4 — Code/documentation follow-up and qualification boundary

## Status and source identity

- Mission: `M-NEM-MIG-20261119-PC-CDS16-01`; checkpoint `UP-C07`; iteration `4`.
- Expected worktree: `bevy_openusd` branch `update-libraries`, baseline HEAD `1aeca866d2271ee6adb0d54f069a112cee7f33e8`, with the **uncommitted I4 changes already present**, paired source fingerprint `ab9f5c95f83dd17aaaab8cf7dd9fa3d5068aa5204ff57b98acd428dfba8b1969` before importing this patch. The fingerprint will change after import.
- Frontend baseline `UsdHubUI@6f5f747d30fc6b8d896859744c6c546401f4fa62`; this follow-up has **no frontend edits**.
- Reviewer I4 result: F-C07-I3-01 **PASS on the paired frozen connected fixture**. C07 S01–S04 qualification remains **BLOCKED by incomplete E-C07-04 evidence**, not by a confirmed new code failure. Configured pre-patch Compile was 8/8 PASS and Local Agent Test was 16/16 PASS; neither result validates this patch.

## What this patch changes (code/documentation only)

1. `crates/usd_bevy/src/live/stage.rs`: new `LiveStage::visual_loadable_paths_if_backed()` returns `Ok(None)` for a documentless Stage, `Ok(Some(paths))` for a successfully queried file-backed visual Stage, and propagates file-backed discovery and synchronization errors. The query uses existing visual Stage synchronization and leaves canonical USD and load policy untouched.
2. `src/viewport/api/bridge/focus_bounds.rs`: `focus_params_for_payload` stops treating any failed native `find_loadable`/visual synchronization as if there were no visual Stage. It returns a correlated error through existing `tree.rs::apply_tree_commands` rejection, avoiding a silent partial Bevy-entity fit. It keeps the existing documentless and no-overlap fallback, canonical unique-anchor fit, and the existing source/world transform mathematics unchanged.
3. `tests/unit/viewport/api/bridge/tests/selection/selection_performance_tests.rs`: extends registered regression tests to check distinct visual context for actual file-backed and documentless Stages. The existing nested unload, occurrence and disconnected fit tests remain intact.
4. This document states the exact code-versus-evidence acceptance boundary for downstream agents. Documentation is not a runtime receipt.

This is a **defensive hardening change**, not a claim that I4 had a demonstrated file-backed discovery failure. It does not create a new project/cache/scheduler or change camera protocol. No source file beyond the above targets is authorized by this patch.

## Checklist disposition — do not invent PASS

| I4 review item | Code/docs disposition after patch import | Execution/evidence still required |
|---|---|---|
| F-C07-I3-01 unique ordinary parent focus | Already corrected before this patch; preserve canonical f64 BBoxCache source-bound routing and single Stage up-axis application | Any future source change requires new matching Compile/Test; the I4 paired images qualify only their exact fixture |
| R-C07-I4-01 file-backed discovery error vs documentless fallback | **Addressed at code-control-flow level** by typed optional result; operational failure rejects rather than silently fitting incomplete entity geometry | Actual connected failure injection and ACK/reject trace not supplied here |
| R-C07-I4-01 occurrence while native load set precedes Bevy projection | No speculative source rewrite: existing occurrence-specific entity fit preserved; no proven failure to repair | Local Agent must test occurrence during visual reconciliation; change source only after reproducible RED |
| E-C07-04 cold first useful frame | No code change justified by current frozen evidence | Cold true displayed-frame timing and generation identity required |
| E-C07-04 selected/clipped authored/generated LOD, image/flicker, seams/materials/holes | Preserve visual/canonical isolation and exact geometry | Connected full visual corpus required |
| E-C07-04 forced native visual-load failure and recovery | Defensive discovery error is now fail-closed; native batch/retry mechanisms not altered | Connected native apply/readback failure injection and bounded-retry trace required |
| E-C07-04 Project Git history/query/edit/commit/export while owner unloaded | No change to Project or source USD; canonical data remains separate | Complete connected Project Git replay and export comparison required |
| E-C07-04 large federation, RSS/VRAM and queues | No speculative limits or new performance code | Large-scene long soak and per-cycle peaks/queues required |
| P-C07-I4-P01 selected GPT-6.1 Sol High profile | No code/doc change can verify UI-selected model/effort | Owner authorization/profile check required |

**Do not mark E-C07-04 or P-C07-I4-P01 PASS after importing code.** These are separate evidence/process gates. A generic patch cannot provide their runtime truth. Do not edit prior Reviewer reports, frozen packet, or the Implementer-authored authoritative working plan as part of import; only the Implementer may update the working plan with true executed outcomes afterward.

## Preflight and import (Local Agent; not run by Reviewer)

From the actual **uncommitted I4 worktree**, inspect `git status --short`, `git rev-parse HEAD`, `focus_bounds.rs`, `stage.rs` and registered selection tests. Do **not** reset, stash, discard or substitute the public GitHub HEAD for the uncommitted target. The I4 `focus_bounds.rs` is Git-visible untracked text in the frozen diff; it must already exist before this incremental patch. Compare the actual file body with the context hunks. If an I3 patch is still unapplied, **stop** and reconcile I3→I4 source first; do not double-apply any earlier focus patch.

```powershell
cd D:\Nemeton\bevy_openusd
$patch = 'C:\path\to\UP-C07-I4_focus_discovery_hardening.patch'
git status --short
git rev-parse HEAD
git apply --check $patch
# Apply ONLY after a clean --check and worktree identity inspection:
git apply $patch
```

If a hunk is already present or `git apply --check` fails, **stop**. Do not use `--force`, `--reject` or automatic whitespace rewriting. Identify exact differing source before deciding whether patch is already implemented or needs a new Owner-authorized revision. The patch is LF/UTF-8 text; preserve bytes on Windows.

## Code verification and regressions (proposed; not executed)

- Compile expectations: `cargo fmt --check`; `cargo check --workspace --all-targets --locked`; the configured Solari/backend/frontend Compile matrix using actual commands from I4 `compile.log` and new source fingerprint. The new public method must compile across `usd_bevy` and viewport usage.
- Local Agent formal Test: repeat the configured 16/16 suite on the **new** source fingerprint; target existing file-backed nested focus and disconnected/occurrence tests, and the registered live/native payload tests. Pre-patch I4 receipts are **not** post-patch PASS.
- Positive: file-backed document opens a visual context and exposes owners even when its nested payload is native-unloaded; focus canonical parent still yields source-complete Z-up result `(0,6,0)` and 22.4 in the known test fixture; source bytes and exports remain unchanged.
- Negative: a documentless `LiveStage::new` returns `None` and keeps ordinary entity focus; a real **file-backed discovery failure** must produce a correlated focus rejection rather than successful partial FlyTo (requires Local Agent injection).
- Boundary: zero overlapping owners in a healthy file-backed context preserves entity fit; loaded and unloaded unique paths preserve the existing canonical/source fallback policy. Occurrence-specific paths use resident entity bounds when safe; projection-lag scenario remains a separate qualification question, not silently resolved by this patch.
- Repeated: focus Near→Far, Stage revision change, cancellation, connected native load/unload while editing/Undo/Redo and selected/section-box use should not cause stale FlyTo or canonical writes.

## Acceptance and handback

The importing agent supplies: (1) exact source diff and worktree fingerprint after apply, (2) `git apply --check` and `cargo fmt --check` results, (3) new configured Compile and Local Agent formal Test logs with fingerprint, (4) focused regression outputs, and (5) explicit E-C07-04 matrix showing NOT_VERIFIED until actual connected receipts arrive. Authoritative plan changes, if any, must be made by the Implementer and truthfully identify the change/limitations. New failures must have a reproducible trace before a separate source patch is authorized.

**No runtime test, product Compile or formal Test was executed by Reviewer to validate this package.**
