# M18-C6 Project renderer regression compatibility

This checkpoint records the renderer compatibility audit for the Project
workflow. Project code remains an authoring and handoff boundary; it does not
replace the existing sparse `StageChangeBatch` projection path with a full
renderer rebuild.

## Scenario matrix

| Scenario | Existing deterministic coverage | Project compatibility result |
| --- | --- | --- |
| Transform-only change | `src/viewport/semantic/tests/changed_info.rs` and `src/viewport/semantic/tests/subtree_delta.rs` | The existing changed-info/subtree delta path remains the consumer; Project mutation only authors the change and drains the batch. |
| Visibility-only change | `src/viewport/scene/visualization_tests.rs` and `src/viewport/semantic/tests/changed_info.rs` | Visibility is reconciled by the existing renderer systems; Project tree state does not own renderer visibility. |
| Material-only change | `src/viewport/scene/visualization_render_mode_tests.rs`, `src/viewport/scene/solari_projection_tests.rs` | Material handles and Solari eligibility remain incrementally reconciled by their existing projection systems. |
| Geometry change | `src/viewport/semantic/tests/render_blob.rs` and `src/viewport/semantic/tests/fallback.rs` | Render-blob enrichment and fallback remain scoped to affected entities; Project activation does not force a geometry rebuild. |
| Nested Scene | `src/project/service/stage_mutation_tests.rs`, `src/project/service/lifecycle_m15_tests.rs`, and `src/project/service/scene_adoption.rs` tests | Nested authoring emits canonical references; the active-stage owner applies them and the normal `LiveStage::drain_change_batch` path observes the change. |
| Repeated Model placement | `src/project/service/model.rs` tests and `src/project/service/stage_mutation_tests.rs` | Placement identity stays distinct while the target Model remains reusable; no renderer-wide reload is requested. |
| PointInstancer fixture | `tests/m8_instancing_correctness.rs`, `tests/m8_instancing_freeze.rs`, and `tests/m10_persistent_soak.rs` | Existing instance identity, prototype dependency and sparse transform reprojection coverage remains independent of Project catalogue reads. |
| Large progressive Scene | `tests/m10_persistent_soak.rs` and the existing progressive-load test support | Progressive delivery and persistent soak continue through the established renderer lifecycle; Project operations do not take ownership of the LiveStage. |

## Source audit

`src/viewport/app/project_stage.rs` reads the private typed mutation outbox,
applies records on the thread that owns `LiveStage`, and explicitly leaves
the resulting `StageChangeBatch` to the normal drain/reconcile systems. It
does not clear the stage, recreate the renderer, or clone the Stage for a
background Project operation.

`src/viewport/app/project_activation.rs` opens a candidate stage before
replacing the current one. A failed activation therefore preserves the
existing renderer state, while a successful activation reuses the established
stage lifecycle and readiness path.

The Project service tests cover nested Scene authoring, repeated placement
identity, inactive Project/Scene outbox isolation, and root transitions. The
renderer tests listed above cover sparse semantic deltas, material/visibility
projection, render-blob fallback, PointInstancer dependencies, and progressive
soak. Together they provide deterministic code-level compatibility evidence.

Hardware/GPU runtime evidence is not part of this checkpoint and remains
unavailable under the standing evidence policy. The acceptance claim here is
limited to source ownership and deterministic test behavior: Project code does
not force a full renderer rebuild where the existing optimized StageChange
handling applies.
