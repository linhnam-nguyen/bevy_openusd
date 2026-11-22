# UP-C06 point picking: bounded work and Section Box correctness

This document describes the selection contracts introduced by the UP-C06
correction patch. The authoritative USD stage remains the source of identity;
GPU draw slots and ECS mesh entities are transient presentation details.

## GPU occurrence query

A single pointer request captures the visible `UsdInstancedDraw` set, source
logical-ID maps, draw transforms and GPU-side copies of the instance, ordered-visible-slot
and count buffers, shared across multipart draws. It partitions the
work into serialized batches. Each GPU submission obeys all three bounds:

- no more than `MAX_DRAWS` result segments;
- no more than `MAX_TRIANGLE_WORK` triangle–visible-row evaluations;
- no more than 65,535 workgroups per compute dispatch dimension.

Even a *single* oversized draw is split into compacted-visible-row windows;
`visible_start` and `visible_limit` in the pick uniform are consumed by both
nearest-depth and nearest-ID shader passes. The source slot returned by the
shader is **not** a slice-local ordinal: it is always the original source
slot from `visible[start + invocation]`.

GPU-side frozen copies are required: merely cloning `Buffer` handles would
read newly compacted slots in later frames and could miss or double-count
occurrences as atomic cull ordering changes. Each asynchronous readback contributes one nearest logical occurrence and
visible-row counts keyed by immutable chunk ID. A finished request publishes
exactly one global nearest hit (distance then stable `(instancer, logical_id)`
tie-break) and counts each chunk's rows once, including across batches and
prototype parts. A request invalidated by pointer, view, scene or
representation revision discards its in-flight traversal; old callbacks
cannot overwrite a newer request. Readback or unsupported-work failures
are **explicit errors**, never successful `hit: None` results. A true no-hit
is only published after all eligible windows finish.

The host must not infer a valid geometric miss from a failed pick. Failure
is logged; the ordinary entity-selection result remains independently
available, and follow-up error handling is a product integration concern.

## Section Box mesh query

Bevy's standard mesh raycast may report only the nearest triangle of each
entity. It is safe for ordinary unmodified projected meshes, but its first
clipped triangle cannot be discarded as if it proved a whole mesh invisible.
Section-clipped mesh entities are therefore excluded from the ordinary query
and traversed with an exact triangle-level query over the retained CPU mesh.
Each triangle is checked against the existing `SectionClipExtension`
world-to-box predicate. The global winner is the nearest *clip-visible*
intersection across clipped and unclipped entities, using world-ray distance.
GPU packed occurrences retain the separate GPU-depth comparison.

Queries use the rendered mesh's topology, winding/culling and world transform,
not synthesized USD data. Non-indexed triangle meshes are supported, and
invalid or unavailable CPU geometry produces explicit diagnostic logging,
not an asserted visibility result. A mesh AABB precheck skips triangle work
for rays outside the projected mesh's local bounds.

`point_box_targets` is conservative projected-bounds selection and is **not**
an exact triangle-visibility oracle; the UP-C06 failure concerns point
hover/click, not a later box-policy expansion.

## Implementation review checks

Before accepting a port, confirm pinned Bevy/renderer APIs for `Indices`,
`Mesh3d`, `SectionClipMaterial`, `MeshAabb` and GPU mapping. On the
Implementer branch, run the configured Compile matrix, the new bounded-pick
unit cases, the same-mesh clip-positive/negative cases, native/remote host
selection regression and 1,024/1,025/1,026 draw boundaries. Do not label
these as executed by the Reviewer; this patch was authored from the frozen
UP-C06 working-tree delta, not from an executable checkout of that delta.
