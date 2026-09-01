# M8-OR3-C9 indexed BIM search

Property-name regex queries now resolve against the snapshot-owned property
dictionary. Object-property matches and replacement previews traverse the
matching property posting list, preserving the existing deterministic bounded
page selector and result ordering. Property-value regex remains a full value
scan because no value-index benefit has been established by a C9 profile.

The query contract and protocol DTOs are unchanged. Missing property names
fail closed with an empty page, and all values still come from the immutable
semantic snapshot through the posting's entity/property positions.
