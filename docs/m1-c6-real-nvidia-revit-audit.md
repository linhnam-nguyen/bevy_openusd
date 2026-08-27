# M1-C6+ real NVIDIA Omniverse Revit Connector audit

Date: 2026-08-27
Repository: `bevy_openusd`
Branch: `develop/panel-BIMData`

## Fixture and provenance

The supplied external fixture is:

`/Users/linhnammac/Documents/000.CodeProjects/0.ReposMac/USDHub/Instance2/external_assets/Omniverse/V2/Projet1.usdc`

- File type: USD crate, version 0.8.0.
- Size: 48,745 bytes.
- SHA-256: `5a145864ded3cad85672d5b000868436a50f35da8e61857092d252f360fe6d6f`.
- Referenced look layer: `V2/Looks.usdc`, SHA-256
  `1e9bacde776302305fee2683e9a9bdb125ac13265d836a5ee5b5372e44634589`.
- Root-layer comment: `Exported from Revit`.
- Root-layer creator: `Revit 2024 via RevitOmniPlugin 203.0`.
- Observable stage metadata: `metersPerUnit = 1`, `upAxis = "Z"`.

The connector's emitted BIM payload is the evidence that BIM data was enabled
for this export: the composed file contains 1,078 `custom string BIM:*`
attributes, split between 438 `BIM:Instance:*` attributes and 640
`BIM:Type:*` attributes, with 119 unique property names. The USD file does not
serialize the connector settings dialog or an `Include BIM Data` boolean, so
the audit claims only the observable export facts above.

## Observed representation

The real export differs from the synthetic C4 fixture in four important ways:

1. BIM data is namespaced in authored custom attribute names, not in a
   `userProperties` dictionary. Representative names are
   `BIM:Instance:Category`, `BIM:Instance:ElementId`,
   `BIM:Instance:IfcGUID`, `BIM:Instance:Surface`, and `BIM:Type:Largeur`.
2. All observed BIM attributes are authored as `string`, including numeric-
   looking values such as `"6000"`, `"200"`, `"22 m²"`, and `"4.47 m³"`.
3. Some values include a localized display unit suffix (`m²` or `m³`), while
   others have no serialized unit sibling or unit identifier. A numeric unit
   cannot be inferred from the property name, display label, or
   `metersPerUnit`.
4. Revit uses prim-level `displayName` metadata for its labels. This is
   distinct from the authored `ui:displayName` attribute read by the current
   OpenUSD UI schema wrapper.

## C6+ extraction evidence

`crates/usd_semantic/src/extractor_tests.rs` contains the ignored external
fixture gate
`real_nvidia_revit_export_properties_reach_semantic_snapshot`. Run it from the
repository root with the supplied assets present:

```text
cargo test -p usd_semantic real_nvidia_revit_export_properties_reach_semantic_snapshot -- --ignored --nocapture
```

The test opens the real composed USD stage, configures the observed
`BIM:Instance:*` names for IFC identity, family, and type ID, and extracts a
`SemanticSnapshot`. It verifies that a real wall entity (`ElementId = 150663`)
has an IFC identity, family `Murs`, type ID `150663`, and preserves
`BIM:Instance:Surface = "22 m²"` and `BIM:Type:Largeur = "200"` as text with
unknown measurement metadata. This proves real connector properties reach the
semantic snapshot without converting or guessing units.

The external test is intentionally ignored in the default suite because the
fixture is supplied workspace evidence outside the `bevy_openusd` repository.
The explicit `--ignored` command above is the C6+ fixture gate.

## Scope decision

No hard-coded Revit property-to-unit mapping was added. The existing explicit
mapping layer remains empty by default and only normalizes a value when a
configured numeric property and an authored sibling unit identifier are both
present. That behavior is compatible with the observed export: its string
values and absent unit identifiers remain lossless, typed semantic properties
with `measurement = None`.

No renderer, frontend, protocol, LiveStage-authoring, or panel behavior was
added. M2 remains unauthorized.
