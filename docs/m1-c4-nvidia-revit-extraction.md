# M1-C4 NVIDIA Revit semantic extraction

Date: 2026-08-27
Repository: `bevy_openusd`
Checkpoint: M1-C4

## Mapping policy

`crates/usd_semantic/src/nvidia.rs` is an explicit, source-specific mapping
layer. It has no Revit SDK or NVIDIA/Kit dependency. A mapping names:

- the authored BIM value property;
- its observed semantic quantity ID; and
- the authored sibling property containing the source unit ID.

The default mapping list is empty. Connector-version and export-setting
differences must be captured from an actual export before a production mapping
is added. The decoder therefore never infers a unit from a property name,
display label, stage unit, or numeric type.

When an explicit mapping resolves a registered source unit, numeric scalar and
numeric-array values are converted to the registry's canonical unit and the
property receives `MeasurementMetadata { quantity, canonical_unit, source_unit
}`. The raw source unit remains available in the sibling semantic property.

Missing, non-text, unregistered, or quantity-incompatible unit metadata leaves
the original typed value unchanged and keeps `measurement` as `None`. This is
the explicit unknown-measurement behavior required for incomplete exports.

## Test evidence

The in-memory semantic fixture in `crates/usd_semantic/src/extractor_tests.rs`
models the decoder contract with `height = 10.0` and
`height_unit = "[ft_i]"`. With an explicit `height -> length` mapping, the
snapshot contains `3.048` in canonical metres and records the source unit.
Separate tests verify that missing and unknown units are preserved without
guessing.

This is synthetic contract evidence, not a claim about the NVIDIA connector
schema. No real NVIDIA Omniverse Revit Connector export is present in this
checkout; connector-specific coverage remains pending the real fixture and its
recorded Include BIM Data/export settings.

## Stage-unit boundary

`metersPerUnit` continues to be consumed only by scene/transform extraction.
It is not used as a fallback for authored BIM parameter units. The extraction
layer remains renderer-neutral after conversion and returns only
`CanonicalValue` plus optional `MeasurementMetadata`.
