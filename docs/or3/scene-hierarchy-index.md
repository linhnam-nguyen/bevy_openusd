# M8-OR3-C6 scene and hierarchy index

## Boundary

The active viewport protocol remains unchanged. C6 changes only the private
backend indexes that serve equivalent `SceneChildrenPage`, hierarchy pages,
visibility lookup, and search reveal metadata.

## Implementation

`SceneAnchorIndex` now publishes a dense scene topology at rebuild time:

- integer node slots for anchors and entities;
- path postings for repeated native-instance occurrences;
- contiguous child-order ranges with sibling positions;
- cold protocol strings retained on the node and cloned only for a response
  page.

`HierarchyPageIndex` uses the same shape for provider-neutral hierarchy rows:
one child-order array and one range per parent, with a hash lookup only for
parent identity resolution. Page reads do not reconstruct a parent map or
scan the complete node vector.

## Complexity contract

After rebuild, the intended query costs are:

| Operation | Cost |
| --- | --- |
| child/sibling page | O(page size) after O(1) range lookup |
| anchor/entity lookup | O(1) expected |
| visibility by anchor/path | O(1) expected / O(occurrences) |
| search reveal ancestry | O(depth) |

Rebuild remains an explicit O(N) publication operation and is triggered only
by projected entity or tree-visible changes.

## Evidence

The `dense_scene_index_pages_and_reveal_metadata_are_bounded` test exercises a
40,000-node synthetic scene and checks deterministic page order, page size,
and reveal-page computation. Its wall-clock output is diagnostic only; no
hardware-specific timing threshold is asserted. Existing native-instance,
visibility, hierarchy paging, and protocol compatibility tests remain the
correctness gate.

Large 100,000-node and live workstation RSS measurements remain C12 evidence
items, not unit-test claims.
