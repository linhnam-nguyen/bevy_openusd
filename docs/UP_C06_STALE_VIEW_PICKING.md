# UP-C06: stale-camera/view click cancellation (F-C06-I3-01)

This correction builds on the I3 reliable button-edge implementation. It does
**not** replace that input queue, GPU batching, or Section Box triangle picker.

## Semantics

A release observed by the renderer is bound to one ray-defining view context:
active camera entity, world and local transforms, current and pending clip
matrices, input viewport size, logical camera viewport dimensions, physical
viewport origin/size, presentation view revision and Section Box pick revision.
The scene and stream generations remain
separate checks. The hover system increments an epoch whenever the observed
view context changes, ensuring an intervening V0 -> V1 -> V0 is not accepted.

The front pending click drives hover/GPU picking **only while** its view stamp,
epoch, stream generation and scene revision match the current view. Selection
rechecks these same conditions immediately before publishing the canonical
`ViewportCommand`. A mismatch discards that click (no fabricated hit, no wrong
selection), and the next queued tap may proceed in a subsequent Update.

Promotion or demotion caused only by selection preserves the click view. It
still invalidates GPU readbacks so the next tap raycasts the current geometry.
Authored reprojection, compatibility changes and promotion-budget fallback
invalidate the view as well as the readback.

## Boundary and explicit limitation

The renderer drops input edges when their first observation detects a changed
view. This conservatively handles input and a camera change processed in the
same update. Hosts supply no displayed-frame token, so an intervening view
change that leaves identical state between observations is undetectable, and
transport delay between browser presentation and input delivery is unmeasured.

## Regression acceptance matrix

1. Same-Update tap; two rapid taps at different coordinates; 1,026 packed draws
   and a newer coalesced cursor: exactly one correct command per valid click.
2. Start V0 pick, orbit/pan/zoom to V1 before GPU completion: no V1 target is
   selected by the V0 click. Repeat V0 -> V1 -> V0 across Update frames.
3. Change projection or logical/physical viewport mapping while waiting:
   discard stale click; no retargeting.
4. Change GPU packing revision, Section Box or scene/stream generation:
   discard stale click; allow a subsequent fresh click.
5. CPU/entity, GPU/packed, clipped rear-triangle, transparent/Solari fallback,
   Ctrl/Command box selection, focus/capture loss and overflow retain their
   existing policies.

The external Rust unit tests check stamp and epoch comparisons. They do not
replace the connected native/remote slow-pick regression and the configured
Compile and formal Test gates, which belong to the Local Agent on the **new**
source fingerprint.
