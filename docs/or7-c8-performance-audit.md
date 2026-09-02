# OR7-C8 root-performance regression audit

Status: complete; structural gate passed. Runtime performance measurement is
unavailable and non-authoritative in this environment.

## Structural checks

- The integrated renderer still uses the root `LiveRevision`/
  `StageChangeBatch` reconciliation path. No older string-owned reconcile plan
  was restored.
- `PrimEntities` remains the root renderer's session-local runtime index. It is
  not used as Project identity; Project and semantic identities remain owned by
  their existing Project/OpenUSD and semantic layers.
- Backend hierarchy paging uses `SceneAnchorIndex` and the dense
  `HierarchyPageIndex` child-order/range index. The frontend Scene Tree uses
  `HierarchyIndex` with precomputed parent, depth, subtree, and virtual-window
  data. No per-row physical hierarchy traversal was introduced.
- BIM and hierarchy workers receive immutable `Arc` snapshots/indexes through
  latest-value mailboxes. No full semantic snapshot clone or unbounded search
  queue was added by OR7.
- Filesystem, Git, and OpenUSD work remains outside the per-frame frontend
  hierarchy path. Classification colorization uses indexed occurrence visits;
  it does not allocate one material per BIM object.

## Scale evidence

- Backend root crate: `cargo test -p usd_bevy` passed, 132 passed, 0 failed,
  1 ignored.
- Backend 40,000-node `dense_scene_index_pages_and_reveal_metadata_are_bounded`
  could not compile through the `usdview` binary because of the inherited
  semantic-sync errors in `src/viewport/semantic/sync/identity.rs` and
  `src/viewport/semantic/sync/mod.rs`. This is a compile-environment limit,
  not a C8 regression diagnosis.
- UI Projects scale fixtures passed 3/3, including 100 Projects and the
  stress fixture with at least 10,000 content nodes.
- UI hierarchy index and virtualization tests passed 11/11.

No nanosecond, FPS, memory, or throughput number is presented as product
evidence. The source-layout gate remains green: 772 backend files scanned, 82
review-band warnings, and 0 files over 400 lines.
