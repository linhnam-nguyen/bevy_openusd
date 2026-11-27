# Logical scene hierarchy

The active composed USD namespace owns logical occurrence identity, names,
parentage, and child completeness. Renderer admission, eviction, and resource
repair only change resident mappings. The logical hierarchy can be empty or
populated independently of GPU resources. Canonical activation replaces the
provisional cached hierarchy even if it already has a published revision.
Canonical authoring, composition and load-rule revisions trigger a coalesced
logical traversal on the Stage owner thread; renderer-only events do not.
Actual canonical deletions remove obsolete logical targets, while eviction
preserves selection and visibility intent for surviving occurrences.

Scene cache schema v3 stores authored hierarchy metadata separately from
transient visibility. The compatibility digest includes the composed hierarchy
qualification contract. Member descendants are captured from the parent
revision's actual composed stage, including occurrence-local overrides and
transparent wrappers. They are metadata only: no child mesh or material bytes
are duplicated. Missing/unloaded or unqualified branches remain Unknown; an
unqualified metadata read does not become a populated default. Unresolved
transparent rows remain expandable rather than collapsing to a false leaf.
Cache-only rows are provisional; canonical traversal supplies the authoritative
namespace before residency. Metadata changes can refresh hierarchy fields
without re-extracting unchanged mesh data.

Visibility commands record generation-scoped subtree roots and command order.
The occurrence forest is captured before transparent presentation rows collapse;
its ancestry, rather than displayed parents, determines the latest applicable
ancestor command. A resolved transparent wrapper can disappear from the tree
without losing its visibility scope or affecting neighboring occurrences.
An absent target under an unresolved branch or provisional cache is deferred,
not authorized as a new command target. Failed extraction withdraws the logical
projection and resident mappings without clearing same-activation intent.
Canonical absence in a resolved branch or a new activation prunes obsolete
intent. Resident bindings and cached occurrence eligibility use the derived
logical visibility, including late-discovered descendants and support rows.
Selection may remain pending while the occurrence is unresolved; it does not
resolve to a stale entity. Eviction does not clear intent. No authored USD
visibility or history is changed.

The frontend retains its separate hierarchy and geometry readiness, correlated
page refresh and expansion reconciliation contracts. No cache-first visual
activation sequencing or wire protocol change is required by this hierarchy
contract.
