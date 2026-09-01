# M8-OR3-C12 integrated performance matrix

Date: 2026-09-01

This is the final OR3 evidence record. A cell is either measured or explicitly
unavailable; no runtime value is inferred from compilation, a CPU mirror, or a
startup-only shader check.

## Matrix

| Workflow | C0 | C12 | Delta |
| --- | --- | --- | --- |
| Hummingbird load | unavailable: no C0 runtime measurement | unavailable: controlled final E2E run remains owner-gated | unavailable |
| Hummingbird idle FPS | unavailable in C0 packet | unavailable: no controlled final E2E run | unavailable |
| Hummingbird playing FPS | unavailable in C0 packet | unavailable: no controlled final E2E run | unavailable |
| playback CPU | unavailable | unavailable: no CPU sampler record | unavailable |
| playback RSS | unavailable | unavailable: no RSS sampler record | unavailable |
| BIM classification cold | unavailable: C0 installed counters only | 32.490 ms, synthetic 4,000-entity profile | not comparable |
| BIM classification warm | unavailable | 3.042 us, same profile | not comparable |
| BIM regex | unavailable | 3.219 ms property-value regex, same profile | not comparable |
| hierarchy root page | unavailable: no scale profile | unavailable: no scale profile | unavailable |
| hierarchy deep page | unavailable: no scale profile | unavailable: no scale profile | unavailable |
| scene search | unavailable: no scale profile | unavailable: no scale profile | unavailable |
| subtree resync | unavailable: no scale profile | unavailable: no scale profile | unavailable |
| semantic update | unavailable: no scale profile | unavailable: no scale profile | unavailable |
| semantic snapshot clones | structural counter installed only | unavailable: no controlled runtime count | unavailable |
| peak RSS | unavailable | unavailable: no process sampler record | unavailable |

The release BIM profile also measured first object-search page `2.506 ms`,
two-target property intersection `82.625 us`, classification colors cold
`0.287 ms`, classification colors warm `296.958 us`, and one classification
build. The profile used 12,000 properties and passed its assertions.

## Integrated implementation evidence

The OR3 sequence is present on local branch `or3/M8-OR3-animation` in the
backend and frontend repositories. Backend checkpoints C0 through C11 are
additive commits; the frontend has C0 and C10 implementation commits. The
complete commit manifest is in `peer-view-portability.md`.

The deterministic gates passed for the implemented paths. The final backend
workspace gate passed in both feature modes:

```text
cargo fmt --all -- --check                       PASS
cargo check --workspace                         PASS
cargo test --workspace                          PASS
cargo check --workspace --no-default-features   PASS
cargo test --workspace --no-default-features    PASS
```

The root `usdview` test target reported `344 passed, 5 ignored`; all workspace
integration targets also passed in the default and no-default runs. Companion
Frost warnings and the macOS linker warning were inherited and non-fatal.
The frontend workspace/WASM gate is recorded in the UI C12 evidence file.

The fixed-16 CPU mirror diagnostic and the temporary Metal startup check show
reference agreement and shader/bind-group compatibility for the candidate.
They do not prove GPU readback parity, equivalent prepass/shadow behavior,
final Hummingbird visual correctness, FPS, CPU, or RSS.

## Owner-gated runtime rows

The following must still be measured by the owner with the fixed-16 candidate
and comparable four-wide control using the same asset, camera, resolution, and
playback settings:

```text
idle:    FPS / CPU / RAM
playing: FPS / CPU / RAM
visual:  body, head feathers, tail transition, wings, other four-wide meshes
render:  projected prim count, prepass, shadows, equivalent-pass FPS
timeline: UI playhead versus backend StageTime
```

The C2 prototype limitation remains material: its custom extended material
needs final runtime confirmation for equivalent prepass and shadow passes.
The timeline playhead correctness path is separate from the animation hot
path and is derived from the authoritative frontend read model.
