# M8-OR3-C7 semantic snapshot ownership

## Boundary

Semantic snapshots remain immutable and the existing bounded state/query and
runtime-delivery mailboxes remain unchanged. C7 changes ownership at their
handoffs; it does not introduce a persistent shared-map implementation.

## Implementation

The authoritative snapshot is now carried as `Arc<SemanticSnapshot>` through:

- sync actions and scoped delta results;
- the semantic worker state mailbox;
- runtime-delivery pending/work items;
- diff working/baseline state.

Replacing a snapshot or retaining a saturation-recovery fallback therefore
increments an atomic reference count instead of copying the entity map. Blob
preparation uses `Arc::make_mut` before the snapshot is published, preserving
owned mutation without making a copy when the action is still uniquely owned.

Scoped changed-info and subtree extraction still copy the entity map once when
they must construct the next complete snapshot. That copy is part of creating
new immutable content; it is not an ownership handoff copy.

## Evidence

The root `usdview` gate passed after the conversion:

- `cargo fmt --all -- --check`;
- `cargo check --bin usdview`;
- `cargo test --bin usdview`: 342 passed, 5 existing ignored.

Semantic mailbox, worker, diff, subtree, runtime-delivery, persistence, and
large BIM tests all remain green. The existing `semantic_snapshot_clones`
diagnostic remains zero because it no longer counts cheap `Arc::clone`
handoffs; live before/after RSS and latency measurements remain C12 evidence.
