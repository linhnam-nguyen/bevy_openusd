# M1-C1 BIM data baseline and source audit

Date: 2026-08-27
Repository: `bevy_openusd`
Implementation branch: `develop/panel-BIMData`
Baseline branch: `server/develop`
Baseline SHA: `68d3deb4645b030a653f5b27beb514c68073180`

## Scope

M1-C1 records the source layout, current semantic data path, baseline gates, and
the evidence available for NVIDIA Omniverse Revit Connector BIM exports. This
checkpoint makes no product-behaviour change and does not infer a connector
schema from a guessed namespace.

## Baseline gates

- `cargo check --workspace --all-targets`: PASS.
- `cargo test -p usd_model -p usd_semantic`: PASS (13 tests, including
  integration tests and documentation tests with no failures).
- `git diff --check`: PASS.
- Rust source-size audit: PASS. No Rust file exceeded 400 lines; the largest
  file was 400 lines. No source-size exception was added.
- The checkout was clean before the checkpoint branch was created.

The baseline check emitted existing warnings from `bevy_frost` on unused items,
deprecated APIs, and an unfulfilled `expect` configuration. They are not M1
changes and are recorded as baseline warnings, not as a passing claim for a
warning-free workspace.

## Source and data-path map

- `crates/usd_model/src/snapshot.rs` owns the renderer-neutral semantic snapshot
  types. `SemanticProperty` currently contains only a name and a canonical
  value.
- `crates/usd_semantic/src/metadata.rs` extracts USD prim metadata and custom
  attributes into source-neutral semantic properties.
- `crates/usd_semantic/src/extractor.rs` computes metadata, entity, and snapshot
  hashes. Property hashing currently covers property names and values.
- `src/viewport/semantic/store/` owns the working in-memory Turso schema and
  delta updates. Its normalized property table currently stores name and value.
- `src/project/semantic_store/` owns durable committed-snapshot persistence and
  migrations. It stores the complete snapshot JSON as well as normalized
  entities and properties.
- `crates/usd_diff/` compares semantic snapshots and currently compares property
  names and values.

## Connector evidence

No real NVIDIA Omniverse Revit Connector USD export is present in this checkout.
The available USD files are generic or synthetic fixtures:

- `crates/usd_semantic/tests/fixtures/identity_original.usda` contains
  source-namespaced identity examples (`source:revitUniqueId`, `source:ifcGuid`,
  and related identifiers), but is not a connector export and is not evidence
  of a BIM parameter schema.
- `crates/usd_semantic/tests/stages/custom_attrs.usda` and
  `custom_attrs_extensive.usda` exercise generic USD custom attributes and
  `userProperties:*`; they do not establish Revit Connector field names or
  parameter units.
- `crates/usd_semantic/tests/stages/physics_units.usda` exercises authored
  `metersPerUnit` stage metadata and is useful for keeping stage units separate
  from property measurement metadata.

NVIDIA's public Revit Connector documentation confirms that the connector
  exports Revit scenes to OpenUSD, that the USD stage unit setting defaults to
  feet, and that `Include BIM Data` exports BIM data. The same settings document
  lists coordinate-source choices (Internal Origin, Project Base Point, Survey
  Point, and Shared Coordinates). It does not provide a complete, machine-
  readable per-parameter quantity/unit schema that can be safely hard-coded in
  this repository. Therefore M1 extraction must preserve unknown measurement
  metadata explicitly and must not guess from a property name or display label.

## M1 modification map

1. M1-C2 adds source-neutral quantity, unit, and measurement metadata types to
   `usd_model`. They remain independent of USD, Revit, NVIDIA, Bevy, Turso, and
   frontend types.
2. M1-C3 adds the tested authoritative unit registry and conversion service in
   `usd_semantic`, with stable string IDs at the semantic boundary and O(1)
   registry lookup after one-time initialization.
3. M1-C4 adds explicit observed-schema extraction hooks. A property receives
   measurement metadata only when the source evidence supplies a recognized
   quantity and unit; otherwise it remains an ordinary property with an
   explicit unknown-measurement state (`None`).
4. M1-C5 propagates measurement metadata through property hashing, working and
   durable persistence, diff comparison, and compatibility tests. Stage
   `metersPerUnit` remains separate from property metadata.
5. M1-C6 records the evidence and freezes the backend-only foundation. A real
   NVIDIA/Revit export remains a required acceptance artifact for claiming
   connector-specific field coverage; this checkpoint does not fabricate that
   artifact.

## Ownership decisions

- `LiveStage` remains authoritative for uncommitted edits; semantic snapshots
  and stores remain derived read models.
- Measurement metadata is semantic data, not renderer state and not UI state.
- No Revit SDK, NVIDIA runtime, OpenUSD runtime type, filesystem path, Git type,
  Turso type, or Leptos type crosses the semantic protocol boundary.
- Unknown quantities and units are represented as unknown rather than guessed.
- Stage units describe scene/transform interpretation and are not silently
  copied onto authored BIM parameters.
