# BOV-C4 Rendering Migration Ledger

Mission: `M-BOV-20261117-PC-CDS16-01`  
Plan revision: `sha256:3f7378770cdbafa638ef443f16e5a0cd60dee69fbaf938a3e6c8b6023c15c56b`  
Checkpoint / iteration: `BOV-C4 / 1`  
Pre-change `bevy_openusd` HEAD: `781956c3009b408ecdaf25a1fbe35571b7517216`

This successor record covers BOV-C4 only. Earlier migration ledgers and both enumeration baselines remain unchanged. The C4 identity map is relative to the committed C3 target. Local Agent composes the existing C1–C3 maps with this map when performing live parity verification; this ledger does not claim that formal Test or parity was run by the Implementer.

## Scope and runtime ownership

Implemented phase 4 of the Owner-selected BOV plan. Rendering presentation, Solari, skeleton debug display, offscreen resize, and render-host cache-presentation observation now live under `viewport::rendering`. Sampling stays in its existing module. The moved presentation and Solari external unit-test/support files moved in this checkpoint as listed below. `UsdHubUI` remained read-only.

No renderer schedules, runtime state contracts, feature gates, plugin registration order, or test bodies were intentionally changed. The app runner still applies stream resize before `ViewportBridgeSet::ApplyCommands`; project activation still installs the same render-subapp cache-presentation systems.

## Production source path mapping

| Previous path | BOV-C4 path |
| --- | --- |
| `src/viewport/scene/visualization/mod.rs` | `src/viewport/rendering/presentation/mod.rs` |
| `src/viewport/scene/visualization/environment.rs` | `src/viewport/rendering/presentation/environment.rs` |
| `src/viewport/scene/visualization/render_mode.rs` | `src/viewport/rendering/presentation/render_mode.rs` |
| `src/viewport/scene/visualization/shadows.rs` | `src/viewport/rendering/presentation/shadows.rs` |
| `src/viewport/scene/visualization/edge/mod.rs` | `src/viewport/rendering/presentation/edge/mod.rs` |
| `src/viewport/scene/visualization/edge/mesh.rs` | `src/viewport/rendering/presentation/edge/mesh.rs` |
| `src/viewport/scene/solari/mod.rs` | `src/viewport/rendering/solari/mod.rs` |
| `src/viewport/scene/solari/projection.rs` | `src/viewport/rendering/solari/projection.rs` |
| `src/viewport/scene/solari/proof.rs` | `src/viewport/rendering/solari/proof.rs` |
| `src/viewport/scene/solari/support.rs` | `src/viewport/rendering/solari/support.rs` |
| `src/viewport/scene/skeleton.rs` | `src/viewport/rendering/debug/skeleton.rs` |
| `src/viewport/app/offscreen_resize.rs` | `src/viewport/rendering/offscreen/resize.rs` |
| `src/viewport/app/cache_presentation_render.rs` | `src/viewport/rendering/offscreen/cache_presentation.rs` |

Added only the module declaration files needed for the new owners: `src/viewport/rendering/debug/mod.rs` and `src/viewport/rendering/offscreen/mod.rs`. `rendering/mod.rs` now declares the rendering owners while retaining `sampling` unchanged.

## External test and support path mapping

All listed attachments retain their Rust `mod_name` and empty `module_path`; the file moves do not add new Rust test-module nesting.

| Previous path | BOV-C4 path | Source owner attachment |
| --- | --- | --- |
| `tests/unit/viewport/app/offscreen_resize_inline_tests.rs` | `tests/unit/viewport/rendering/offscreen/resize_tests.rs` | `src/viewport/rendering/offscreen/resize.rs` |
| `tests/unit/viewport/scene/solari_tests.rs` | `tests/unit/viewport/rendering/solari/tests.rs` | `src/viewport/rendering/solari/mod.rs` |
| `tests/unit/viewport/scene/solari_projection_tests.rs` | `tests/unit/viewport/rendering/solari/projection_tests.rs` | `src/viewport/rendering/solari/mod.rs` |
| `tests/unit/viewport/scene/solari_projection_asset_tests.rs` | `tests/unit/viewport/rendering/solari/projection_asset_tests.rs` | `src/viewport/rendering/solari/mod.rs` |
| `tests/unit/viewport/scene/solari_projection_test_support.rs` | `tests/unit/viewport/rendering/solari/projection_test_support.rs` | `src/viewport/rendering/solari/projection.rs` |
| `tests/unit/viewport/scene/visualization_tests.rs` | `tests/unit/viewport/rendering/presentation/tests.rs` | `src/viewport/rendering/presentation/mod.rs` |
| `tests/unit/viewport/scene/visualization_environment_tests.rs` | `tests/unit/viewport/rendering/presentation/environment_tests.rs` | `src/viewport/rendering/presentation/environment.rs` |
| `tests/unit/viewport/scene/visualization_render_mode_test_support.rs` | `tests/unit/viewport/rendering/presentation/render_mode_test_support.rs` | `src/viewport/rendering/presentation/render_mode.rs` |
| `tests/unit/viewport/scene/visualization_render_mode_tests.rs` | `tests/unit/viewport/rendering/presentation/render_mode_tests.rs` | `src/viewport/rendering/presentation/render_mode.rs` |
| `tests/unit/viewport/scene/visualization_shadows_test_support.rs` | `tests/unit/viewport/rendering/presentation/shadows_test_support.rs` | `src/viewport/rendering/presentation/shadows.rs` |
| `tests/unit/viewport/scene/visualization_shadows_tests.rs` | `tests/unit/viewport/rendering/presentation/shadows_tests.rs` | `src/viewport/rendering/presentation/shadows.rs` |

The ten moved test/support bodies were preserved. The Solari projection support helper changed only its restricted visibility path from `crate::viewport::scene::solari` to `crate::viewport::rendering::solari`; assertions and test function bodies were not edited. `scripts/test_architecture_policy.json` now names the eleven moved owner/target attachment pairs and the moved Solari production dependency.

## Test identity correspondence

The source owner moved, so unit-test module prefixes change. Within each target-qualified entry, the function suffix below is retained exactly once. Every listed identity occurs in both `Cargo.toml` target `usdview` kinds (`bin` and `lib`), one occurrence per kind. The tables therefore report two target-qualified entries per logical test identity.

### Default-feature identities

| Previous target-qualified prefix | BOV-C4 target-qualified prefix | Test function identities retained | Logical names | Target-qualified entries |
| --- | --- | --- | ---: | ---: |
| `viewport::app::offscreen_resize::tests` | `viewport::rendering::offscreen::resize::tests` | `fps_only_stream_update_preserves_offscreen_dimensions`; `rgba_byte_count_is_width_times_height_times_four` | 2 | 4 |
| `viewport::scene::solari::tests` | `viewport::rendering::solari::tests` | `solari_capability_requires_compile_device_and_scene_support`; `stable_capability_publication_does_not_dirty_viewer_settings`; `supported_capability_is_published_as_renderer_neutral_viewer_state` | 3 | 6 |
| `viewport::scene::visualization::environment::tests` | `viewport::rendering::presentation::environment::tests` | `background_projection_updates_clear_color_only`; `fallback_surface_projection_updates_fallback_state_without_authored_materials`; `grid_projection_applies_color_visibility_and_world_origin_without_spawning`; `scene_origin_tracks_loaded_geometry_when_selected` | 4 | 8 |
| `viewport::scene::visualization::render_mode::tests` | `viewport::rendering::presentation::render_mode::tests` | `render_mode_scans_only_on_transitions_and_new_uniform_meshes`; `uniform_color_rebinds_and_shaded_restores_each_authored_material`; `uniform_color_tracks_material_route_changes_before_restoring` | 3 | 6 |
| `viewport::scene::visualization::shadows::tests` | `viewport::rendering::presentation::shadows::tests` | `shadows_disable_globally_and_restore_each_authored_setting` | 1 | 2 |
| `viewport::scene::visualization::tests` | `viewport::rendering::presentation::tests` | `edge_mesh_deduplicates_shared_triangle_edges_and_rejects_points`; `edge_overlay_is_independent_from_wireframe_for_all_four_combinations`; `full_renderer_configuration_matrix_preserves_each_independent_option`; `gizmo_size_level_uses_log_scale_for_both_gizmo_paths`; `grid_visibility_reads_the_authoritative_renderer_configuration`; `render_mode_round_trip_updates_bevy_wireframe_without_touching_edges` | 6 | 12 |
| **Default total** |  |  | **19** | **38** |

### Solari-feature-only identities

These eight identities are enabled only in the `solari` feature configuration. They are separate from the three default Solari identities above.

| Previous target-qualified prefix | BOV-C4 target-qualified prefix | Test function identities retained | Logical names | Target-qualified entries |
| --- | --- | --- | ---: | ---: |
| `viewport::scene::solari::projection_asset_tests` | `viewport::rendering::solari::projection_asset_tests` | `mesh_asset_event_dirties_indexed_consumer` | 1 | 2 |
| `viewport::scene::solari::projection_tests` | `viewport::rendering::solari::projection_tests` | `explicit_dirty_entity_is_processed_incrementally`; `shaded_idle_does_not_repeat_full_projection_scan`; `stable_ray_traced_does_not_repeat_full_projection_scan`; `supported_usd_meshes_get_raytracing_markers_without_material_mutation`; `unchanged_large_scene_processes_zero_projection_entities_on_idle`; `unsupported_usd_projection_is_diagnosed_and_never_marked_for_raytracing` | 6 | 12 |
| `viewport::scene::solari::tests` | `viewport::rendering::solari::tests` | `camera_and_proof_mesh_follow_ray_traced_mode_and_restore_shaded` | 1 | 2 |
| **Solari-only total** |  |  | **8** | **16** |

The complete C4 map covers 27 logical names and 54 target-qualified entries across the default and Solari-feature configurations. Test support attachments contribute no test identity. The historical `scripts/test_enumeration_baseline.json` and `scripts/test_enumeration_qualified_baseline.json` are unchanged; no baseline was regenerated. This map must be composed with the prior checkpoint maps for live migration parity.

## Compatibility, visibility, feature, and CI disposition

- `scene::visualization` remains a crate-visible re-export of `rendering::presentation`, preserving scene-facing consumers while establishing rendering as the implementation owner. `scene::SolariCapability`, `scene::SolariCapabilityPlugin`, and skeleton runtime items remain re-exported from the new Solari/debug owners. Sampling remains under `rendering::sampling`.
- `scene::projection_work` remains scene-owned. Its module and three shared items move from scene-only visibility to `pub(in crate::viewport)` so presentation and Solari can continue to use the shared projection-work abstraction; no crate-external API is exposed. The edge mesh helper is limited to `pub(in crate::viewport::rendering::presentation)`.
- Rendering `offscreen` modules are visible only within `viewport`. The cache-presentation `install` hook is `pub(in crate::viewport)` so stage activation can reach it from its sibling subtree. App runner and project activation call the same functions at the same setup points.
- The `solari` Cargo feature gate remains unchanged. CI's CPU-safe Solari filter and its CI workflow expectation were both updated from `viewport::scene::solari` to `viewport::rendering::solari`.
- No C4-owned shader asset path or embedded shader reference required relocation. Existing section-box embedded shader registration remains called from presentation at the same point.
- No Cargo target registration, `UsdHubUI` source, or later-phase source/test owner changed.

## Verification and limitations

The first Solari `-- --list` selector attempt exited 101 before enumeration, with eight compile errors: six `E0425` errors for classification-color and selection-policy names formerly resolved through the old `scene` parent, plus two `E0742` errors because render-mode and shadows test-support stats still declared `pub(in crate::viewport::scene)` after moving outside that ancestor. Pre-move `HEAD` inspection confirmed `scene/mod.rs` supplied the former classification/selection reexports and the support structs were scoped to the old scene tree. The correction imports those existing scene-owned items into presentation and narrows each support struct to its new render-mode/shadows owner. The raw failed diagnostic is preserved at `AgentsComm/communication/agents/implementer/bov_c4_solari_list_diagnostic.txt`.

After the correction, this enumeration-only command passed with exit 0 and resolved the configured selector to 11 live Solari identities, `11 tests, 0 benchmarks`:

`cargo test --message-format short -p usdview --lib --features solari --locked viewport::rendering::solari -- --list`

This command listed identities and did not execute test bodies. Its captured output, including existing compiler warnings, is at `AgentsComm/communication/agents/implementer/bov_c4_solari_list_diagnostic_retry.txt`. The diagnosis and fix were also recorded in `.agent/memory.md`.

The first configured Compile attempt stopped at its initial `cargo fmt --all -- --check` command (exit 1); no subsequent configured command ran. Rustfmt reported only import-order/wrapping and module-declaration ordering differences in eight authorized C4 Rust files. `cargo fmt --all` completed successfully, and the worktree contains only authorized C4 paths. A final configured Compile retry is pending.

Formal Test, live parity, hosted CI execution, native/GPU rendering smoke, commit, and push are not Implementer actions for this checkpoint.
