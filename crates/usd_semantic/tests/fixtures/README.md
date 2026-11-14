# Identity fixtures

`identity_moved.usda` is a crate-local deterministic contract fixture paired with
the shared `tests/fixtures/usd/identity_original.usda` fixture, not an export
captured from Revit or Omniverse. Together they record the candidate property
names used by the resolver test:
`source:revitUniqueId`, `source:ifcGuid`, `source:applicationGuid`, and
`source:assetIdentifier`.

The real Revit/Omniverse fixture required by the implementation plan is still
an evidence-gathering task: once an exporter-generated file is available, its
actual property names should be added to `IdentityConfig` and tested here
before enabling them as application defaults.
