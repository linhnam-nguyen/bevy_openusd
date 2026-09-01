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

## Additive C2+ / C12+ animation repair

The owner E2E launch exposed a real Metal pipeline validation failure in the
original candidate: the forward fixed-16 vertex layout was being reused by
Bevy's prepass and shadow pipelines, while those pipelines still consumed
Bevy's prepass locations. The repair is additive and preserves the complete
C0-C12 history:

```text
M8-OR3-C2+  297dde7  pass-correct extended skin prepass
M8-OR3-C12+ evidence update and runtime startup gate
```

C2+ keeps the forward shader and its locations unchanged, adds a dedicated
fixed-16 prepass/deferred vertex shader, and selects the matching Bevy
prepass/shadow input locations during specialization. Bevy's shadow path uses
the depth-prepass key, so the same contract covers both prepass and shadows.
The four extended influence groups remain packed at locations 8 through 13,
and motion-vector previous-skin support is retained in the custom prepass
shader.

C2+ deterministic gates passed:

```text
cargo fmt --all -- --check              PASS
cargo check -p usd_bevy -p usdview      PASS
cargo test -p usd_bevy --lib            PASS — 113 passed, 1 ignored
git diff --cached --check               PASS
```

C12+ runtime startup evidence on Apple M4/Metal passed the former failure
point for the same Hummingbird asset:

```text
cargo run -p usdview --bin usdview -- assets/external/hummingbird.usdz \
  --headless --transport webrtc --preset adaptive --codec h265
  PASS — process reached and held the runtime without pbr_prepass_pipeline
         or shader-location validation errors; stopped with SIGINT

cargo run -- assets/external/hummingbird.usdz
  PASS — native windowed process reached the runtime without the former
         pbr_prepass_pipeline or shader-location validation errors; stopped
         with SIGINT
```

The prior prototype limitation about intentionally disabling prepass and
shadows is superseded by C2+ for this candidate. The automated launches prove
Metal shader/pipeline startup compatibility only; they do not prove GPU
readback parity, visual correctness, measured FPS/CPU/RSS, equivalent render
pass cost, or timeline playhead behavior.

## Additive C4+ compact path-index repair

The first consolidated OR3 owner review rejected C4 because the original
implementation retained path strings in `PrimEntities`, native-instance
dependencies, and PointInstancer dependencies, including duplicated ancestor
prefix postings. C4+ replaces that architecture with a shared `PathStore`
that owns each canonical path string once and returns stable compact `PathId`
values to the indexes.

`PrimEntities` now stores `PathId` mappings plus a compact parent-to-children
topology for subtree traversal. Native-instance and PointInstancer indexes
store only `PathId` edges; ancestor queries walk interned IDs and no longer
construct or retain per-index prefix strings. Full projection cleanup clears
the indexes and the path store together, while scoped reconciliation removes
edges without requiring a stage-wide path-index rebuild.

C4+ deterministic gates passed:

```text
cargo fmt --all -- --check              PASS
cargo check -p usd_bevy -p usdview      PASS
cargo test -p usd_bevy --lib            PASS — 116 passed, 1 ignored
git diff --check                        PASS
```

The C4+ tests include shared-ancestor ownership, topology removal, and
PointInstancer compact-ID dependency resolution. This repair does not include
the rejected-review follow-ons C5+ (compact ChangePlan) or C10+ (hierarchy
clone/ancestry work).

## Additive C4++ native dependency lookup repair

Owner Review found one remaining C4+ hot-path defect: native
`dependents_for_path()` still scanned every registered prototype after its
direct ancestor lookups. C4++ removes that scan. Ordinary exact or property
changes now resolve only the changed path and its interned ancestors through
direct `HashMap<PathId, ...>` lookups. The bidirectional prototype/root
relationship scan is retained only in `dependents_for_resync_root()`, where it
is required for structural composition boundaries.

The regression proves that a nested exact change still reaches its proxy,
that a parent exact change does not scan down into descendant prototypes, and
that the same parent used as a structural resync root still reaches the proxy.

C4++ deterministic gates passed:

```text
cargo fmt --all -- --check                      PASS
cargo test -p usd_bevy --lib exact_lookup_avoids_descendant_prototype_scan
                                               PASS — 1 passed
cargo test -p usd_bevy --lib                    PASS — 117 passed, 1 ignored
cargo check -p usd_bevy -p usdview              PASS
git diff --cached --check                       PASS
```

## Additive C4+++ native descendant dependency repair

The consolidated Owner Review found that C4++ had removed the ordinary
`by_prototype` scan but had also removed a required USD namespace semantic:
an inheritable property change on a prototype ancestor must reach registered
descendant prototype consumers. C4+++ preserves the bounded lookup by adding
one shared compact namespace topology to `PathStore`.

`PathStore` now retains parent-to-children `PathId` edges alongside its single
canonical path-byte table. Native-instance ordinary dependency lookup first
checks the changed path and its interned ancestors, then traverses the changed
path's compact namespace descendants and performs direct `HashMap<PathId, ...>`
lookups. It does not scan `by_prototype` and does not reintroduce duplicated
prefix-string postings. Structural resync lookup uses the same complete
ancestor-plus-descendant traversal, preserving both sides of the composition
boundary without a reverse-map scan.

The C4+++ regression proves that an ordinary inheritable ancestor change
reaches a nested registered prototype proxy, excludes an unrelated prototype
branch, and preserves exact nested and structural-resync dependency coverage.

C4+++ deterministic gates passed:

```text
cargo fmt --all -- --check
  PASS
cargo test -p usd_bevy --lib ordinary_lookup_preserves_descendant_prototype_dependencies
  PASS — 1 passed
cargo test -p usd_bevy --lib
  PASS — 117 passed, 1 ignored
cargo check -p usd_bevy -p usdview
  PASS — pre-existing Frost warnings only
git diff --check
  PASS
```

The repository `make harden` command was also invoked. It stopped at its
pre-existing source-size audit before compile/test/clippy stages because
`crates/usd_bevy/src/live/native_animation.rs`,
`src/viewport/api/scene_index.rs`,
`src/viewport/api/bridge/scene_query.rs`, and
`src/viewport/api/scene_query.rs` exceed the 400-line hard limit. No C4+++
file exceeds that limit, and no unrelated oversized file was modified.

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

The remaining owner gate is visual/performance E2E comparison against the
four-wide control. The timeline playhead correctness path is separate from the
animation hot path and is derived from the authoritative frontend read model.
