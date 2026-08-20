# Identity fixtures

These are deterministic contract fixtures, not exports captured from Revit or
Omniverse. They record the candidate property names used by the resolver test:
`source:revitUniqueId`, `source:ifcGuid`, `source:applicationGuid`, and
`source:assetIdentifier`.

The real Revit/Omniverse fixture required by the implementation plan is still
an evidence-gathering task: once an exporter-generated file is available, its
actual property names should be added to `IdentityConfig` and tested here
before enabling them as application defaults.
