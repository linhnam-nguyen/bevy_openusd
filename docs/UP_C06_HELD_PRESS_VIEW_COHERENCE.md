# UP-C06: held-press camera/view coherence (F-C06-I4-01)

This source correction extends `docs/UP_C06_STALE_VIEW_PICKING.md`. It closes the
**press-to-release** stale-view path, including the synchronous Ctrl/Command box
selection branch. It does **not** change the previously corrected queued-release
picking behavior, WebRTC message schema, GPU batch traversal, or USD authority.

## Failure and corrected invariant

Before this correction a press under V0 could survive an Update with a changed
camera V1 but **no button edges**, because the click state only discarded that
press on a view-changed Update *with* new edges. A release on a later stable V1
Update could select a V1 point or form a V0-origin/V1-end box gesture.

`StampedPress` now owns the primary down edge and the observed view stamp,
monotonic view epoch and scene-index revision. Each `select_point_click` Update,
even with zero input edges, rejects a stale press when any of the following
changes: observed camera/view context, view epoch, input generation, scene
revision, or input viewport size. Release rechecks the full contract and both
edge generations/viewport sizes **before either** point-intent queuing or box
selection. On mismatch the handler emits no selection command and never
translates the old press into the new camera's coordinate system.

The click's view is the one first observed by the renderer; this is not a
cryptographic or event-time proof of the image displayed by the receiver.
When a press/release arrives in the same Update as a detected view change,
the existing fail-closed I4 rule still discards ambiguous edges.

## Invariants deliberately preserved

- Ordered `PrimaryButtonEdge` queue and reliable CSS-pixel snapshot transport.
- Ctrl/Command modifier at press, existing 4 px distance threshold, and bounded
  `MAX_SELECTION_TARGETS` box selection through `ViewportCommandInbox`.
- `QueuedPointClick` + GPU readback correlation, logical `SceneAnchor` IDs,
  one-select-per-valid-click semantics, and >1,024 draw batching.
- Focus/UI capture, stream reset, queue overflow, Section Box and transparent/
  Solari CPU fallback semantics; no authored USD mutation.
- Existing test identities and registration ledger; add assertions to the
  already registered external hover test rather than changing enumeration.

## Acceptance handoff (recommended, not run by Reviewer)

Implementer/Local Agent should verify: (1) valid down and up at a stable V0
across multiple Updates selects exactly once; (2) Ctrl/Command moved box at
stable V0 yields the same bounded target set; (3) V0 press, no-edge V1 camera
change, V1 release yields zero point/box selections; (4) V0->V1->V0 changes
still cancel; (5) scene/stream/viewport changes and focus/capture clear press;
(6) a fresh valid tap after cancellation still works; (7) 1,026 packed draws
and fast consecutive taps remain exact-once. Capture connected CPU and packed
GPU traces, then apply the normal Implementer Compile and Local Agent formal
Test gate against the **new** combined source fingerprint.

Residual boundary: there is still no native/remote displayed-frame token.
Changes between renderer observations that leave the same final observed view
cannot be distinguished by this patch. This requires separate Owner-approved
scope if product policy demands true displayed-frame causality.
