# Incremental Scene cache qualification

The Scene cache is derived data. A persisted delta is an optional optimization,
not authority to publish a generation or a substitute for canonical USD.

## Source intervals

`LiveStage` retains a bounded authored notice log independently of visual
reconciliation. The log starts at the canonical root bytes captured on opening
or an independently qualified successful save. Canonical save failure does not
clear it. Load-rule, muting and recomposition notices invalidate the baseline;
reopening provides a new sealed source. The visual resolver remains sealed to
its original source and is never relabeled after a save.

Before a managed save, the cache owner opens the actual canonical Scene and
compares its root against the notice baseline, and its non-root source bytes and
resolver associations against the sealed live opening. The retained current
cache pointer is not a source baseline. After a successful save, while authoring
is still leased, the cache owner checks the saved root's serialized content and
stage context against the active authored stage and verifies the dependency
closure. Unqualified saves still succeed and enqueue ordinary full preparation.
A change in stage-level composition context uses full rebuild. If qualification
cannot advance the persisted baseline, reopen the document before attempting
another delta; full preparation remains available in the meantime.

Each qualified hint contains exact base and result cache revision digests.
Queued intervals merge only if both are complete and adjacent. An existing job
with unknown history is not equivalent to an absent job. The worker must match
the result to its sealed target, load and verify the exact immutable base, and
check compatible schema, projection recipe and target identity. Missing,
corrupt, incompatible or discontinuous intervals use clean full preparation.
Exact-target verified reuse and the existing generation/retirement/pin/atomic
publication transaction remain unchanged.

## Structural changes

A structural resync may remove or replace a Material, Shader, Skeleton,
reference or other dependency consumed outside the changed subtree. Inspecting
only the resulting stage does not establish the old dependency closure. Until
both sides can be qualified with a complete dependency graph, all structural
resyncs (including additions, removals and renames) use the existing full
builder. This is deliberately conservative; ordinary BIM metadata,
transform, localized geometry, material and animation property edits retain
the sparse path. A future narrower structural path must prove clean-full
parity, including external consumers of removed or replaced resources.

The revision schema digest includes an incremental interval qualification tag.
Previously retained descriptors from the unqualified implementation cannot
pass exact-target reuse under the new contract. Shared CAS payload formats and
canonical files are unchanged; valid bytes may still be reused by content hash.
