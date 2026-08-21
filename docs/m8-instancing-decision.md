# M8 PointInstancer decisions

Status: implementation complete; awaiting the M8 milestone review.

## C3 — automatic batching gate

Decision: **NO CUSTOM INSTANCING RECOMMENDED**.

The current implementation keeps one Bevy entity per visible logical row and
shares prototype `Mesh` and fallback `StandardMaterial` handles. M8 has no
renderer extraction, draw-call, GPU-memory, or renderer-batch artifact that
would justify a custom batch path. The release C1 projection artifact is
headless (`MinimalPlugins`), so it is evidence for projection/entity/allocation
behavior only. The inherited M4 renderer packet remains the available native
renderer evidence and does not include a controlled PointInstancer custom
instancing comparison.

Adding a renderer-specific batch layer without that comparison would make
logical selection, live edits, invisible IDs, and reprojection harder to prove
while changing no measured bottleneck. The existing `ProjectionCache` and
source-mesh cache are the approved data-plane optimization.

## C4 — GPU-native senior gate

Decision: **NO CHANGE RECOMMENDED; SENIOR GATE NOT OPEN**.

There is no M8 before/after renderer evidence meeting the gate. The code keeps
logical identity in `UsdInstanceId` (`index` plus `prototype_index`) rather
than coupling selection to a future renderer instance index. A GPU-native
PointInstancer path remains a separately gated design task requiring transform,
multiple-prototype, invisible-ID, picking, live-change, reload, and framing
proof before implementation.

## C5 — logical identity and visibility contract

Visible instance entities carry `UsdInstance` and `UsdInstanceId`. The ID is
the source row, not the compacted visible-row position. `invisibleIds` is
resolved through authored `ids`; only when IDs are unauthored is the value
treated as a row index. Hidden rows are not spawned, so they cannot render or
be picked through an instance entity. Reprojection clears only prior instance
children and preserves shared assets.

The regression fixture and release tests cover two prototype indices, stable
logical ordering, authored `invisibleIds`, reprojection without asset minting,
and a changed prototype producing a distinct mesh cache entry. SceneAnchor
ownership remains renderer-neutral; no frontend or Instance 2 change is part of
M8.
