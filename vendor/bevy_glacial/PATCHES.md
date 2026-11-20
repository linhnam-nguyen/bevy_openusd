# Vendored `bevy_glacial` (Bevy 0.20.1 / WESL Transitional Adaptation)

## Provenance
- **Source repository**: `../bevy_glacial` (`https://github.com/bresilla/bevy_glacial`, fork at `d:\Nemeton\bevy_glacial`)
- **Pinned baseline commit**: `6bef365be227e1461c1f8019e3b8c631c166fd44`
- **Reason vendored in UP-C01**:
  - Sibling repository `../bevy_glacial` targets Bevy `0.19.x` and is outside the authorized write scope of Mission `M-NEM-MIG-20261119-PC-CDS16-01` UP-C01 (`bevy_openusd` + `UsdHubUI`).
  - Active `bevy_openusd` viewport and tests (`GroundGridPlugin`, `AxisGizmoPlugin`, `TransformGizmoPlugin`, `ChaseCamera`, `GizmoAutoScale`, `LEVEL_HALF`, etc.) depend on `bevy_glacial`.
  - Vendoring into `vendor/bevy_glacial` (excluded from `[workspace].members`) keeps a single unified `bevy 0.20.1` + `glam 0.30` dependency graph while preserving `../bevy_glacial` untouched.

## Applied Modifications
1. `Cargo.toml`: Updated `bevy` & `bevy_math` to `0.20.1`, `glam` to `0.30`, `ecolor`/`emath`/`epaint` to `0.35`, `ahash` to `0.8`, `uuid` to `1.17`.
2. `src/gizmo/render.rs`: Added `#[extract_app(RenderApp)]` for Bevy 0.20 `ExtractComponent`, added `constants: Default::default()` to `VertexState`/`FragmentState`, and updated `TemporaryRenderEntity` construction to `TemporaryRenderEntity::default()`.
3. `src/selection_ring.rs` & `src/shaders/selection_ring.wesl`: Migrated `selection_ring.wgsl` to WESL (`import bevy_pbr::...`, `@if(PREPASS_PIPELINE)`, `@group(constants::MATERIAL_BIND_GROUP)`, updated 3-argument `alpha_discard`).

## Licenses
- `bevy_glacial`: `MIT` (declared in `Cargo.toml`, `license = "MIT"`).
- Inlined `transform-gizmo` / `transform-gizmo-bevy` (`src/gizmo/`): dual-licensed `MIT OR Apache-2.0` (`LICENSE-MIT.transform-gizmo`, `LICENSE-APACHE.transform-gizmo`).

## Removal / Reconciliation Criteria
- Remove `vendor/bevy_glacial` when `UP-C02-S04` completes either:
  - upstream/sibling `bevy_glacial` migration to Bevy `0.20.1` (allowing a pinned git/registry dependency), OR
  - replacement/re-scoping of Glacial camera/gizmo dependencies in `bevy_openusd`.
