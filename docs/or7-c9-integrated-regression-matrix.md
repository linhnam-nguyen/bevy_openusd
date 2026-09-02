# OR7-C9 integrated Projects root regression matrix

Status: complete; focused integration matrix passed. The backend workspace
compile limitation remains recorded exactly and is not attributed to C9.

## Matrix

| Contract | Evidence | Result |
| --- | --- | --- |
| Root renderer and semantic snapshot invariants | `cargo test -p usd_bevy --quiet` | 132 passed, 0 failed, 1 ignored |
| Typed viewport and BIM command wire contracts | `cargo test -p viewport_protocol --quiet` | 73 passed, 0 failed |
| Project activation and direct/refresh lifecycle source coverage | Existing `project_activation` and `scene_index_lifecycle` tests | Source coverage present |
| 40,000-node dense hierarchy boundedness | `cargo test --quiet --bin usdview scene_index -- --nocapture` | Blocked before execution by inherited semantic-sync compile errors |

## Boundary checks

- Project activation remains rooted in the existing session lifecycle and
  authoritative event path; this checkpoint added no alternate scene owner.
- Scene Tree hierarchy identity, paging, and reveal metadata remain separate
  from BIM classification presentation.
- BIM classification uses the typed recipe command and preserves the root
  contextual hierarchy contract.
- No Revit, Omniverse, live browser, GPU, native Tauri, WebRTC, or production
  proof is claimed by this CPU-side matrix.

The blocked backend binary compile reports the inherited `has_resync` and
`resync_roots` API mismatch in `src/viewport/semantic/sync/identity.rs` and
`src/viewport/semantic/sync/mod.rs`. It is an environment/repository baseline
limitation for this checkpoint, not evidence of a C9 regression.
