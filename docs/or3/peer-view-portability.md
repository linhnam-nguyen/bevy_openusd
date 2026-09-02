# M8-OR3 portability manifest for Projects peerView

Date: 2026-09-02

This manifest classifies the OR3 changes for a later forward-port. No
PeerView branch was modified during OR3. The later transfer must adapt each
portable behavior to the receiving branch's architecture rather than blindly
cherry-picking the whole sequence.

## Checkpoint classification

| Checkpoint | Classification | Transfer note |
| --- | --- | --- |
| M8-OR3-C0 | PORTABLE evidence/instrumentation | Counter boundaries are reusable where the receiving runtime has equivalent ownership points. |
| M8-OR3-C1 | PORTABLE | Typed animation runtime isolation; preserve the native stage boundary. C1+ prebinding is an additive correction to the same boundary. |
| M8-OR3-C2 | PORTABLE | Standard4/Extended16 fidelity classification and transform-only playback; renderer pass integration must be adapted and revalidated. |
| M8-OR3-C3 | PORTABLE | Root-local projection working-set planning. |
| M8-OR3-C4 | PORTABLE | Compact path and dependency indexes. C4++++ adds the production-path native-instance regression and caller migration without changing the compact-ID architecture. |
| M8-OR3-C5 | PORTABLE | Sparse dependency reconciliation. C5+ applies the compact PathId ChangePlan to the batch-local work representation. |
| M8-OR3-C6 | PORTABLE | Dense scene and hierarchy index model. |
| M8-OR3-C7 | PORTABLE FOUNDATION | Immutable semantic snapshot ownership and Arc sharing. |
| M8-OR3-C8 | BIM-SPECIFIC | Snapshot-owned BIM field/classification indexes. |
| M8-OR3-C9 | BIM-SPECIFIC | Indexed BIM property-name search; value regex remains an evidence-gated scan. |
| M8-OR3-C10 | SPLIT | Hierarchy ordering/windowing and timeline projection are portable; C10+ bounds the frontend hierarchy projection with revision-bound Arc storage. BIM panel-specific projection remains BIM-specific. |
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
M8-OR3-C12 95cb2b11a131872926ff8735369e11e4e351014d

Additive backend corrections:

```text
M8-OR3-C2+      297dde7493f4e52c1188a41a91271fa48e26ebd8
M8-OR3-C12+     23fc9c9024764f94a9014a6bc61af1ed93701910
M8-OR3-C4+      2d3d469e249cecaf7cc117f7d6a558dd59c0f576
M8-OR3-C4++     a308faeb418ae92cebedca28df8bf12479e2abc3
M8-OR3-C4+++    8afaee4a5876cea06e120976f592367b13e2e0cb
M8-OR3-C4++++   c839fe5b51382dcf54765cc5f216519ce992cf17
M8-OR3-C1+      863ef1c4b8584059f26035ad6d9a12467123b9aa
M8-OR3-C5+      7d3c2daf8355dda5de0ec60c648243deb32e6cd0
M8-OR3-C12++    3c904b1b75900cd8280a19fa0822d73ab35b1535 (retained history only)
M8-OR3-C12+++   documentation closure (this additive commit)
```

Frontend `UsdHubUI`, local branch `or3/M8-OR3-animation`:

```text
M8-OR3-C0  ea5d0baf713a064fd6ffee65410319fd88359b30
M8-OR3-C10 bacf71ad45ff6bc1302961c2bc1500f799817b9a
M8-OR3-C12 a621cc66388f301f05135533789033d8c53da3cd
```

Additive frontend correction:

```text
M8-OR3-C10+ 95036e7ea9f4b8a10d3cdcfafc91fc383b5c9517
```

The active frozen remote baselines remain backend
`5b1810b13e8b64d300065600887ae1a2a70e09cf` and frontend
`e5d58e7cadd595a8e7f5cb2d2ba7328fefe175d5`. The temporary fixed-16
worktree and its `.usdhub/` state remain uncommitted and are not a merge
source.

## Consolidated correction boundary

The authorized additive correction pass was completed in this order without
an Owner Review pause:

```text
M8-OR3-C4++++ → M8-OR3-C1+ → M8-OR3-C5+ → M8-OR3-C10+
```

These commits are evidence for review, not a freeze or merge authorization.
The temporary fixed-16 worktree, frozen OR2 branches, and PeerView branches
remain outside the implementation scope. Final Hummingbird E2E FPS/CPU/RAM,
visual, equivalent render-pass, GPU readback, and timeline StageTime evidence
remains owner-gated and is not inferred from deterministic tests.

## C12+++ closure / frozen source basis — 2026-09-02

`M8-OR3-C12+++` is documentation/evidence-only. It removes the production
diagnostic changes introduced by C12++ and restores the backend production
source to the approved C6+ basis. C12++ remains in git history only and is not
the frozen production source. No PeerView branch was modified.

```text
Backend frozen source basis:
d3873de6b1fdf04383814cc1c12fa0c7e80615a0 + C12+++ documentation closure

Frontend frozen head:
be4c603420a28013f2a87a79ea738fa8056fd443
```

Owner Review:

```text
Controlled Hummingbird FPS / CPU / RAM comparison waived because the
review host must remain under concurrent video-streaming workload.
```

No rendering-performance claim is made. OR3 architecture/correctness is
accepted and frozen; no C2++ is required. The authorized next operation after
final Owner Review is merge OR3 into `develop`. The temporary fixed-16
worktree, frozen OR2 branches, and PeerView branches remain outside this
closure.
