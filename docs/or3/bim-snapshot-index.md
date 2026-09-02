# M8-OR3-C8 BIM snapshot-owned indexes

`SemanticSyncState` now publishes one immutable `Arc<BimReadIndex>` with each
accepted semantic snapshot. The index owns deterministic BIM entity order,
prim-path lookup, a property-name dictionary with entity/property postings, and
the model-wide classification field catalogue. The snapshot remains the source
of truth for semantic values and protocol strings.

Classification indexes are memoized by recipe inside the same snapshot-owned
index. Services created for bridge reads, hierarchy projection, color planning,
and the search worker therefore share the classification build rather than
rebuilding it per request. `BimReadService::new` remains a standalone fixture
constructor; production paths use `with_index` from `SemanticSyncState`.

The C8 tests establish that the shared classification cache has one entry per
recipe and that property postings cover the deterministic BIM entity order.
