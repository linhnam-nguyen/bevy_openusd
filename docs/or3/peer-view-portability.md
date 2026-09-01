# M8-OR3 portability manifest for Projects peerView

Date: 2026-09-01

This manifest classifies the OR3 changes for a later forward-port. No
PeerView branch was modified during OR3. The later transfer must adapt each
portable behavior to the receiving branch's architecture rather than blindly
cherry-picking the whole sequence.

## Checkpoint classification

| Checkpoint | Classification | Transfer note |
| --- | --- | --- |
| M8-OR3-C0 | PORTABLE evidence/instrumentation | Counter boundaries are reusable where the receiving runtime has equivalent ownership points. |
| M8-OR3-C1 | PORTABLE | Typed animation runtime isolation; preserve the native stage boundary. |
| M8-OR3-C2 | PORTABLE | Standard4/Extended16 fidelity classification and transform-only playback; renderer pass integration must be adapted and revalidated. |
| M8-OR3-C3 | PORTABLE | Root-local projection working-set planning. |
| M8-OR3-C4 | PORTABLE | Compact path and dependency indexes. |
| M8-OR3-C5 | PORTABLE | Sparse dependency reconciliation. |
| M8-OR3-C6 | PORTABLE | Dense scene and hierarchy index model. |
| M8-OR3-C7 | PORTABLE FOUNDATION | Immutable semantic snapshot ownership and Arc sharing. |
| M8-OR3-C8 | BIM-SPECIFIC | Snapshot-owned BIM field/classification indexes. |
| M8-OR3-C9 | BIM-SPECIFIC | Indexed BIM property-name search; value regex remains an evidence-gated scan. |
| M8-OR3-C10 | SPLIT | Hierarchy ordering/windowing and timeline projection are portable; BIM panel-specific projection remains BIM-specific. |
| M8-OR3-C11 | PORTABLE where backend core | Evidence-only residual decision; any later mesh change requires a new profile. |
| M8-OR3-C12 | EVIDENCE ONLY | Matrix, gate, ownership, and portability records; no PeerView implementation. |

## Local commit manifest

Backend `bevy_openusd`, local branch `or3/M8-OR3-animation`:

```text
M8-OR3-C0  9885c6fb39934ae2c6e1cc3ef47579b99262cb3c
M8-OR3-C1  7c016f045c65f9291fb8c64ac3a718b08a8e748c
M8-OR3-C2  612ee63bb8eb8a368dbaf016e7f4e04bfbde9f0c
M8-OR3-C3  f3ac3719fb947b72515b2b4ece456ef1d675ce9d
M8-OR3-C4  84079ce1ed3df891ff1fe2d76524db0bb3e4719b
M8-OR3-C5  5f05f049dde442930efddaf82fb1e3e7df642563
M8-OR3-C6  74ba138d1dd7728812acc3cf8c4a440fb394abf9
M8-OR3-C7  4fc468d38d143c81030fc34353e8f75f86bbdc1f
M8-OR3-C8  681425bc36bb84278e76b63b82170767d34d6090
M8-OR3-C9  27f6f1623eca89ab076aebb57c67699cd1931b3e
M8-OR3-C11 6c8a704a2b1eb6a247026d4d0ba558f7796f3801
M8-OR3-C12 <this checkpoint after final gate>
```

Frontend `UsdHubUI`, local branch `or3/M8-OR3-animation`:

```text
M8-OR3-C0  ea5d0baf713a064fd6ffee65410319fd88359b30
M8-OR3-C10 bacf71ad45ff6bc1302961c2bc1500f799817b9a
M8-OR3-C12 <this checkpoint after final gate>
```

The active frozen remote baselines remain backend
`5b1810b13e8b64d300065600887ae1a2a70e09cf` and frontend
`e5d58e7cadd595a8e7f5cb2d2ba7328fefe175d5`. The temporary fixed-16
worktree and its `.usdhub/` state remain uncommitted and are not a merge
source.
