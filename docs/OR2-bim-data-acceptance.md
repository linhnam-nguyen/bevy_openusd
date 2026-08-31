# M8-OR2 — BIM Data Acceptance Packet

Date: 2026-08-31

Status: `IMPLEMENTED / OWNER REVIEW 2 REQUIRED — C4++++/C5++++/C6+/C7++++/C7+++++/C7++++++ COMPLETE`

Review boundary: Owner Review 2. OR2 is complete at the implementation and
automated-evidence boundary recorded here, including the additive
C1+/C3+/C4+/C5+/C7++ repair batch, the C4++/C5++ correction, and this
C4++++/C5++++/C6+/C7++++ review repair, the additive C7+++++ provider
restoration correction, and the additive C7++++++ test-layout correction. It
is not marked `PASSED / FROZEN`
until the owner reviews this packet.

## Branch continuity and scope

OR2 is a continuation of the existing panel branches. No separate OR2 branch
was created or retained.

- Backend: `develop/panel-BIMData`
- UI: `panel-BIMData`
- The pre-existing UI `Cargo.lock` modification remains uncommitted and was
  preserved.
- The backend pre-existing `.codex/` directory remains untracked and was not
  staged.

The milestone covers bounded BIM property delivery, stale request handling,
classification color and hierarchy behavior, authoritative field/property
controls, panel flex/scroll behavior, production semantic-adapter startup,
model-wide BIM field-catalogue delivery, normalized Revit classification
semantics, compact property presentation, and the integrated acceptance matrix.

## Checkpoint ledger

| Checkpoint | Backend | UI | Result |
| --- | --- | --- | --- |
| M8-OR2-C1 | `0789264` | `243412e` | Bounded BIM property pages/chunks, correlation, reassembly, and explicit oversized-value errors. |
| M8-OR2-C2 | — | `9fc3ba4` | Idle/loading/ready/error lifecycle, supersession, cancellation, and stale-page rejection. |
| M8-OR2-C3 | `2db05a2` | `2164e04` | Typed classification color path, authored-color restoration, Auto refresh, and native-instance occurrence fan-out. |
| M8-OR2-C4 | `533bc75` | `fc43ae8` | Deterministic BIM virtual hierarchy, real scene anchors, `<Unclassified>`, paging, and virtual-group selection boundary. |
| M8-OR2-C5 | — | `0b588c5` | Authoritative field/property controls, validation, revision invalidation, and preserved classification interactions. |
| M8-OR2-C6 | — | `178c816` | Built-in flex/scroll contract, long-value wrapping, compact rows, and source/test module sizing. |
| M8-OR2-C7 | `83972b3` | `178c816` | Initial integrated gates, realistic-load audit, acceptance packet, and Owner Review 2 stop. |
| M8-OR2-C1+ | `dbb99d0` | `a279b12` | Near-linear bounded property-page packing, focused frontend property-state/reducer modules, and 4,096-property scale evidence. |
| M8-OR2-C3+ | `dbb99d0` | — | Indexed scene-occurrence reverse lookup, indexed classification fan-out, and 4,000-occurrence scale evidence. |
| M8-OR2-C7+ | `3799bbd` | `a279b12` | Prior acceptance record corrected: `dbb99d0` remains implementation-only; `3799bbd` is the backend acceptance-packet commit. |
| M8-OR2-C4+ | `eec3aa2`, `4ef5132` | — | Normalized BIM-identity eligibility boundary, explicit semantic-adapter configuration for BIM integration fixtures, and mixed hierarchy/search/color regression. |
| M8-OR2-C5+ | — | `5a5a713` | `Classification [ON]` editor with automatic row numbering, searchable human-facing field catalogue, trailing placeholder promotion, delete/reorder actions, and one active color row. |
| M8-OR2-C7++ | `4ef5132` + packet update | `5a5a713` | Corrected acceptance bookkeeping, reran the final gate matrix, recorded C4+/C5+ evidence and final code refs, pushed both continuous branches, and reopened Owner Review 2. |
| M8-OR2-C4++ | `fddda20` | — | Normal `usdview` startup now injects the observed NVIDIA/Revit semantic adapter; BIM eligibility also rejects whitespace-only identity evidence. |
| M8-OR2-C5++ | `fddda20`, `b432d1e` | `262d926` | Prior protocol-v8 bounded model-wide BIM field catalogue, semantic-revision publication/idempotence, selection-independent UI reduction, and stale-revision rejection. |
| M8-OR2-C7+++ | packet commit below | — | Final backend/UI gates, supplied asset audit, plan/packet update, continuous-branch push, and Owner Review 2 stop. |
| M8-OR2-C4++++ | `6674aae` | — | Preserved generic OpenUSD `kind`/`type_name` semantics and added explicit normalized Revit Category/Family/Type classification sourced only from verified fields. |
| M8-OR2-C5++++ | `6674aae` | `5e3776c` | Protocol-v9 model-wide descriptor catalogue with human labels, `[I]`/`[T]` scope, deterministic byte-bounded paging, exact reconstruction, stale rejection, and no partial UI catalogue. |
| M8-OR2-C6+ | — | `5e3776c` | Exact Search/Classification/Properties layout, compact one-line properties, raw-key titles, scope badges, and removal of Selection/legacy status presentation. |
| M8-OR2-C7++++ | final packet commit (recorded in the implementation plan) | — | Final correction gates, asset/runtime limitation record, implementation-plan update, continuous-branch push, and Owner Review 2 stop. |
| M8-OR2-C7+++++ | `99594be` | `4343944` | Reversible Prim ↔ BIM provider switching: authoritative remote Prim root reload after BIM return, repeated backend/UI round trips, and stale inactive-provider page rejection in both directions. |
| M8-OR2-C7++++++ | — | `d493d0b` | Extracted the hierarchy-provider fixture into a focused module, split the remote regression into provider-specific cases, moved the pre-existing interaction cases out of the oversized aggregate test module, and kept every materially touched Rust test file under 400 lines. |

## C7 implementation and repair evidence

The C7 integration pass made only bounded acceptance-supporting repairs:

- Consolidated BIM property page metadata into a typed `PageContext`, keeping
  the paging helper below the strict argument-count lint and preserving the
  encoded-size boundary.
- Added the public `is_empty` companion required by the native dependency
  index API lint.
- Split the BIM property delivery tests from the chunk test module so the
  source-size audit remains at zero files over 400 lines.
- Added the page/error event arms to the read-model no-op match and repaired
  deterministic classification test reordering for the current map API.
- Replaced per-prefix page cloning and serialization with cached per-item JSON
  sizes plus conservative incremental page accounting. The final page remains
  checked against the authoritative envelope at page boundaries.
- Split the frontend property lifecycle/assembly and property-event reduction
  responsibilities into `properties_state.rs` and `property_reducer.rs`.
- Added `SceneOccurrenceIndex` and routed path-only classification lookup through
  it, so each color entry resolves its actual scene occurrences without scanning
  every indexed entity or allocating a result vector.

### Repair scale evidence

```text
M8-OR2-C1+ paging scale: properties=4096 pages=67 elapsed_ms=90.948
M8-OR2-C3+ indexed color scale: entities=4000 path_lookups=4000 occurrence_visits=4000 elapsed_ms=22.625
```

The C1+ implementation serializes each property and empty-group shape once,
then accumulates bounded size deltas while building pages. The final page is
still validated against the authoritative envelope. The C3+ regression
observes one indexed path lookup and one occurrence visit per unique entry;
the existing duplicate-path regression continues to cover multi-occurrence
fan-out semantics.

## C4+/C5+ contract-repair evidence

### C4+ authoritative BIM eligibility

`SemanticInfo::is_bim_entity()` is the source-neutral eligibility fact consumed
by the classification/search boundary. It accepts only non-empty normalized
`bim.element_id` or `bim.family_name` evidence produced by an explicitly
configured semantic adapter. Generic USD category/type/display metadata and
arbitrary `BIM:*` property names do not opt an entity into BIM projection.

`ClassificationIndex::build()` and the BIM search entity iterator now consume
only eligible entities. Eligible BIM entities with a missing selected field
still project under `<Unclassified>`; cameras, lights, helpers, assemblies,
and plain meshes remain outside the BIM hierarchy, search result universe, and
classification color entries.

The mixed regression covers two eligible windows (including one missing its
selected category) plus Camera, Light, Helper, and PlainMesh entities. It
proves the non-BIM anchors are absent from hierarchy and color output and that
`PropertyNameRegex("^NonBimOnly$")` returns zero matches. The semantic bridge
integration fixture now explicitly configures `BIM:Instance:ElementId`, so
existing BIM edit/classification convergence remains a valid normalized-adapter
test rather than relying on generic metadata.

```text
cargo test -p usd_model: 12 passed
cargo test -p usdview classification_contract: 1 passed
cargo test -p usdview --bin usdview live_edit_converges_into_bim_classification_search_and_diff: 1 passed
```

### C5+ frozen classification editor

The UI presents `Classification` with an explicit `[ON]`/`[OFF]` toggle. The
assigned rows are automatically numbered and expose human-facing `Category`,
`Family`, `Type`, and authoritative BIM property names through a searchable
native field catalogue. Internal IDs such as `property-4` are not rendered.
There is exactly one trailing `Parameter name...` placeholder; selecting a
catalogue value promotes it to an assigned typed `ClassificationLevel` and
leaves the next placeholder available. Removal and reordering operate on the
typed recipe while row numbering is recomputed from the visible order. Color
selection remains zero-or-one active row and existing transport dispatches are
preserved.

```text
cargo test -p usd_hub_desktop: 232 passed; 1 ignored
cargo test --workspace --all-targets (UI): desktop 232 passed/1 ignored; viewport_client 11 passed
```

## C4++/C5++ final repair evidence

### C4++ production semantic adapter

The normal `usdview` startup path now inserts
`SemanticSyncState::with_config(SemanticConfig::for_nvidia_revit_connector())`
before the viewport bridge plugin starts. The adapter explicitly maps the
observed NVIDIA/Revit identity property `BIM:Instance:ElementId`; it is not a
test-only setter and classification does not hardcode exporter property names.
The semantic BIM-eligibility predicate also treats whitespace-only element IDs
as absent, so a valid family identity can still qualify an entity.

The bridge integration test app uses this same runtime adapter constructor and
asserts that the initial semantic snapshot contains the normalized element ID.
The former `configure_bim_runtime_semantics` test-only override was removed.

```text
cargo test -p usd_semantic nvidia_revit_runtime_adapter_uses_observed_element_id_property: 1 passed
cargo test -p usdview live_edit_converges_into_bim_classification_search_and_diff: 1 passed
cargo test -p usd_model whitespace_only_normalized_identity_is_not_eligibility_evidence: 1 passed
```

### C5++ model-wide catalogue

Protocol version 8 carries `BimClassificationFieldCatalogue`, bounded to
4,096 fields. The backend derives it from the accepted semantic snapshot over
all BIM-eligible entities, always retaining typed `Category`, `Family`, and
`Type` fields, and deterministically adding validated property names. It is
independent of `SelectionReadModel` and selection-scoped property paging; a
property found only on a Window remains available while a Wall is selected,
while helper-only properties remain excluded.

Semantic synchronization publishes one catalogue event when the accepted
semantic revision/list changes. Snapshot and reload responses include the
current catalogue. The UI resets to typed defaults during stage loading,
reduces the model-wide event without a selection request ID, rejects stale
revisions, and projects the catalogue regardless of selection or property
response state.

```text
cargo test -p usdview field_catalogue_is_model_wide_bim_only_and_revision_scoped: 1 passed
cargo test -p usdview identical_catalogue_revision_is_published_only_once: 1 passed
cargo test -p usd_hub_desktop model_wide_field_catalogue_is_reduced_without_selection_and_rejects_stale_revision: 1 passed
cargo test -p usd_hub_desktop classification_catalogue_is_independent_of_selection_and_properties: 1 passed
```

## C4++++/C5++++/C6+ additional repair evidence

### C4++++ normalized Revit classification

The generic OpenUSD semantic fields remain source-neutral: `SemanticInfo.category`
continues to represent `prim.kind()` and `SemanticInfo.type_name` continues to
represent `prim.type_name()`. BIM classification no longer reuses those generic
fields. It is carried in the separate `BimClassificationInfo` value and is
populated only through the configured NVIDIA/Revit adapter.

The observed export fields are preserved as authoritative source keys:
`BIM:Instance:Category`, `BIM:Type:Name`, `BIM:Instance:ElementId`,
`BIM:Instance:Commentaires`, `BIM:Instance:Identifiant`,
`BIM:Instance:IfcGUID`, `BIM:Instance:Longueur`, and
`BIM:Type:Description`. Category and Type are normalized from the first two
verified mappings. Family remains absent because no actual family field was
verified; the implementation does not guess one from Category, Type, or an
arbitrary property name.

Property descriptors classify only explicit `BIM:Instance:` and `BIM:Type:`
namespaces as instance/type. Other names retain the `Other` scope. The backend
read model carries the stable raw key, human label, and scope so the UI does
not need to infer BIM meaning from presentation text.

```text
cargo test -p usd_semantic classification_mapping_keeps_generic_usd_fields_separate: 1 passed
cargo test -p usdview classification_contract: 1 passed
cargo test -p usdview property_read_classifies_observed_instance_namespace_without_guessing_values: 1 passed
```

### C5++++ paged descriptor catalogue

The descriptor catalogue now travels on protocol v9. It is model-wide and
semantic-revision-scoped, bounded to 4,096 fields, and uses a deterministic
page envelope whose encoded application message is at most 12 KiB. The
catalogue assembler validates page count, total count, revision, page index,
duplicate keys, and exact reconstruction before replacing the visible field
list. Stale revisions and duplicate pages are ignored; incomplete catalogues
remain hidden and cannot partially replace the prior visible catalogue.

```text
cargo test -p viewport_streaming oversized_bim_classification_catalogue_is_paged_and_reconstructs_exactly: passed
cargo test -p usd_hub_desktop paged_catalogue_stays_hidden_until_all_pages_arrive_and_ignores_duplicates: passed
cargo test -p viewport_protocol --tests: passed
```

### C6+ exact BIM panel presentation

The panel now presents Search, Classification, and Properties in that order.
The BIM search uses the shared `model-view-input` and
`model-view-input__clear` hooks with placeholder `Search BIM data…`. The
Selection card and `BIM properties ready.` status are removed. Properties use
the exact `▼ Properties`/`▶ Properties` header and `Edit` checkbox, render
compact one-line rows without `<Ungrouped>`, show `[I]` or `[T]` only when the
typed scope requires it, and keep the raw property key in the title attribute.

```text
cargo test -p usd_hub_desktop --all-targets: 236 passed; 1 ignored
cargo check -p usd_hub_desktop --all-targets: passed
```

### C7+++++ reversible hierarchy-provider restoration

The remaining Owner Review 2 defect was the remote BIM → Prim return path.
Changing the UI cache source to Prim previously restored only the compact
snapshot, which is intentionally roots-only for a remote session; it did not
request authoritative Prim roots. The correction keeps local fixtures on the
existing snapshot materialization path, while remote sessions invalidate the
old provider cache and queue a fresh Prim root page request. The cache already
rejects any `HierarchyChildrenPage` whose source is not the active provider,
so late BIM pages after a Prim switch and late Prim pages after a BIM switch
cannot repopulate the visible hierarchy.

Backend commit `99594be` adds a deterministic Prim → BIM → Prim projection
round-trip repeated ten times, including the real Prim node names and BIM
classification group. UI commit `4343944` adds local fixture round-trips
repeated ten times and a remote transport/cache regression covering both
authoritative Prim root restoration and late inactive-provider pages.

```text
cargo test -p usdview --bin usdview classification_provider_switch_round_trips_to_prim_repeatedly -- --nocapture: 1 passed
cargo test -p usdview --bin usdview --tests: 336 passed; 5 ignored
cargo test -p usd_hub_desktop fixture_hierarchy_provider_switch_round_trips_to_prim_repeatedly -- --nocapture: 1 passed
cargo test -p usd_hub_desktop remote_bim_to_prim_requests_authoritative_prim_roots -- --nocapture: 1 passed
cargo test -p usd_hub_desktop inactive_bim_page_is_ignored_after_switch_to_prim -- --nocapture: 1 passed
cargo test -p usd_hub_desktop inactive_prim_page_is_ignored_after_switch_to_bim -- --nocapture: 1 passed
cargo test -p usd_hub_desktop --all-targets: 241 passed; 1 ignored
cargo fmt --all -- --check: PASS on both continuous branches
```

### C7++++++ test-layout and bookkeeping correction

Owner Review 2 also identified that the new hierarchy regression had enlarged
the already oversized aggregate UI test module and had pushed the remote test
module over the strict 400-line source limit. This checkpoint changes test
organization only; no production hierarchy logic was modified.

- `store/tests/hierarchy_provider.rs` now owns the local fixture round-trip,
  the authoritative remote BIM → Prim root request case, and the two stale
  inactive-provider cases.
- `store/tests/interaction.rs` owns the three pre-existing interaction tests
  moved out of the aggregate module; their assertions are unchanged.
- `store/tests.rs` and `store/tests/remote.rs` retain their existing
  responsibilities and are now below the source-size limit.

The final materially touched UI Rust test files are bounded as follows:

```text
store/tests.rs: 335 lines
store/tests/remote.rs: 267 lines
store/tests/hierarchy_provider.rs: 235 lines
store/tests/interaction.rs: 120 lines
```

```text
cargo fmt --all -- --check: PASS
cargo test -p usd_hub_desktop hierarchy_provider -- --nocapture: 5 passed
cargo test -p usd_hub_desktop remote -- --nocapture: 4 passed
cargo test -p usd_hub_desktop interaction -- --nocapture: 3 passed
cargo test -p usd_hub_desktop --all-targets --quiet: 241 passed; 1 ignored
```

UI commit `d493d0b` records this source-layout correction on the continuous
`panel-BIMData` branch. The production hierarchy implementation remains the
reviewed `4343944` revision.

## Integrated evidence matrix

### Real supplied asset

Expected command:

```text
cargo test -p usd_bevy --lib --no-default-features live::tests::native_instance_audit::projet1_usdc_windows_project_to_scene_proxy_meshes -- --ignored --nocapture
```

Observed result:

```text
asset-unavailable: ../external_assets/Omniverse/V2/Projet1.usdc is absent on this host
UNAVAILABLE — HARDWARE LIMITATION
```

The historical OR1-C8 CPU audit recorded 5 window roots, 10 proxy meshes, and
10 projected `Mesh3d` entities, but it was not rerun for this correction because
the supplied asset is not present in the current workspace or home-directory
search scope. No new asset-specific screenshot or live `usdview` rendering
claim is made here.

### Backend gates

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets` | PASS |
| `cargo test --workspace --all-targets` | PASS: usdview 335 passed/5 ignored; usd_bevy 111 passed/1 ignored; viewport_streaming 57 passed; remaining workspace tests passed. |
| `cargo check --workspace --all-targets --no-default-features` | PASS |
| `cargo test --workspace --all-targets --no-default-features` | PASS: usd_bevy 111 passed/1 ignored; usdview 334 passed/5 ignored; viewport_streaming 56 passed; all doctests passed or were explicitly ignored. |
| `./scripts/check_rust_file_size.sh` | PASS: 578 files scanned, 0 over 400 lines, 48 warnings in the 351–400 range. |
| Changed-crate strict clippy | PASS for `viewport_streaming`; `usdview` remains baseline-limited by the same four pre-existing findings in untouched API files. No finding was reported for the new packer/index modules. |
| Workspace strict no-default clippy | BASELINE-LIMITED: four findings in untouched `usdview` files (`bim_provenance`, `bim_commands`, and `scene_query`). |
| `make harden` | ENVIRONMENT-LIMITED after source-size, default/no-default checks, workspace tests, and doctests: all-features compile requires unset `DLSS_SDK` and reports Vulkan APIs configured out; underlying cargo failure is `bevy_render`, `make` exit 101. |

### UI gates

The reviewed functional UI revision is `4343944` on `panel-BIMData`; the
prior C5++ revision `262d926` remains in its ancestry. The additive
C7++++++ source-layout correction is `d493d0b`.

- `cargo fmt --all -- --check`: PASS.
- Focused/full `usd_hub_desktop` library tests: 241 passed, 1 ignored.
- UI workspace default and no-default compile/test gates: PASS; desktop
  241 passed/1 ignored in the final post-extraction run.
- UI source-size audit: the materially touched test files are bounded at
  `tests.rs` 335 lines, `tests/remote.rs` 267 lines,
  `tests/hierarchy_provider.rs` 235 lines, and `tests/interaction.rs` 120
  lines. The previously repaired `state.rs` 288 lines, `reducer.rs` 349
  lines, `properties_state.rs` 181 lines, and `property_reducer.rs` 129 lines
  remain unchanged. Other pre-existing oversized UI files were not part of
  this correction and were not touched.
- Strict all-features UI clippy remains baseline-limited by pre-existing
  findings outside the C5+ classification files (including `model_view.rs`,
  benchmark/settings code, and existing scene/store tests); no finding was
  reported in the C5+ classification controls, actions, view-model, or their
  new tests.

## Acceptance coverage

The recorded workspace suites cover the requested stale-request, classification
color, hierarchy, renderer configuration, scroll/panel, idle-activity,
performance, and bounded-delivery surfaces. Representative passing tests
include:

- stale paged-property and stale-property request rejection;
- classification color restoration, repeated Auto refresh, and path-only
  fan-out across native-instance scene occurrences;
- deterministic virtual hierarchy/provider switching, reversible Prim restore,
  bidirectional stale-page rejection, and non-selectable virtual-group
  boundaries;
- bounded BIM property paging and explicit oversized-property errors;
- 4,096-property paging scale with 67 bounded pages and no per-prefix rebuild;
- 4,000-entry indexed classification scale with 4,000 path lookups and 4,000
  occurrence visits;
- large BIM fixture cold/idle query, intersection, and color-cost checks;
- model-wide BIM field-catalogue reduction with no selection, Window-only field
  retention, helper-property exclusion, and stale-revision rejection;
- normalized Category/Type classification with generic OpenUSD semantic-field
  separation, verified namespace scope descriptors, exact paged-catalogue
  reconstruction, and hidden-until-complete UI assembly;
- compact BIM panel layout/search/property presentation and absence of the
  removed Selection/legacy-ready/`<Ungrouped>` presentation;
- renderer configuration, transport, and selection projection acceptance
  tests.

## Evidence boundary

Automated tests prove the authored data path, bounded transport/read-model
behavior, CPU-side projection, hierarchy/classification semantics, and
configuration state transitions. This run also cannot prove the supplied
`Projet1.usdc` asset-specific runtime path because the asset is unavailable.
The tests do not prove a live GPU frame, Tauri window rendering, WebRTC
playback, or H265 output.

Manual visual/runtime evidence is therefore:

```text
UNAVAILABLE — HARDWARE LIMITATION
```

Owner Review 2 should include the supplied launch command and the real
`Projet1.usdc` asset in a capable native GPU/Tauri/WebRTC environment before
OR2 is frozen. No OR3 work is authorized until this Owner Review 2 boundary is
accepted.
