#!/usr/bin/env python3
"""Validate structural M10 performance invariants from machine-readable artifacts."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read JSON artifact {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"artifact is not a JSON object: {path}")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def number(value: Any, name: str) -> float:
    require(isinstance(value, (int, float)) and not isinstance(value, bool), f"{name} is not numeric")
    return float(value)


def check_matrix(path: Path, minimum_fps: float) -> list[str]:
    report = read_json(path)
    require(report.get("passed") is True, f"renderer matrix failed: {path}")
    cases = report.get("cases")
    cadence = report.get("cadence_samples")
    require(isinstance(cases, list) and len(cases) == 16, f"matrix case count is not 16: {path}")
    require(isinstance(cadence, list) and len(cadence) == 3, f"cadence case count is not 3: {path}")
    for case in cases:
        require(case.get("accepted") is True, f"requested/effective renderer mismatch: {path}")
        require(case.get("configuration_matches") is True, f"configuration mismatch: {path}")
    for sample in cadence:
        summary = sample.get("summary", {})
        require(
            summary.get("effective_renderer_target_fps") == sample.get("requested_fps"),
            f"cadence effective/requested mismatch: {path}",
        )
        actual = summary.get("actual_rendered_fps")
        require(actual is not None and number(actual, "actual_rendered_fps") >= minimum_fps, f"FPS gate failed: {path}")
    return [f"matrix {path.name}: 16/16 renderer states, 3/3 cadence states"]


def check_idle_report(path: Path) -> list[str]:
    report = read_json(path)
    grid = report.get("incident_grid", {})
    semantic = report.get("incident_semantic", {})
    require(report.get("configuration_matches") is True, f"idle configuration mismatch: {path}")
    require(report.get("steady_state_matches") is True, f"idle steady-state mismatch: {path}")
    require(grid.get("structural_rebuilds") == 0, f"idle grid structurally rebuilt: {path}")
    require(grid.get("compute_extent_calls") == 0, f"idle extent recomputed: {path}")
    require(semantic.get("idle_skips", 0) > 0, f"idle semantic fast path was not observed: {path}")
    require(semantic.get("snapshot_clones") == 0, f"idle semantic snapshot cloned: {path}")
    return [f"idle {path.name}: no grid rebuild/extent scan and no snapshot clone"]


def check_edit_report(path: Path) -> list[str]:
    report = read_json(path)
    grid = report.get("incident_grid", {})
    semantic = report.get("incident_semantic", {})
    require(grid.get("compute_extent_calls", 0) > 0, f"edit did not recompute extent: {path}")
    require(grid.get("prims_scanned", 0) > 0, f"edit did not scan current prims: {path}")
    require(semantic.get("snapshot_clones", 0) > 0, f"same-session edit did not clone prior snapshot: {path}")
    require(semantic.get("fallback_extractions", 0) > 0, f"full fallback edit was not observed: {path}")
    return [f"edit {path.name}: extent, snapshot clone, and full-fallback paths observed"]


def check_recovery_report(path: Path) -> list[str]:
    report = read_json(path)
    semantic = report.get("incident_semantic", {})
    require(semantic.get("recovery_checkpoints", 0) > 0, f"recovery checkpoint was not observed: {path}")
    require(semantic.get("subtree_extractions", 0) > 0, f"scoped subtree extraction was not observed: {path}")
    require(semantic.get("fallback_extractions") == 0, f"unexpected full reconcile in scoped edit: {path}")
    return [f"recovery {path.name}: checkpoint and scoped extraction without fallback"]


def check_geometry_edit(path: Path) -> list[str]:
    report = read_json(path)
    operations = report.get("operations", [])
    require(isinstance(operations, list), f"geometry operations are missing: {path}")
    geometry_rebuilds = [
        operation
        for operation in operations
        if operation.get("operation") in {"points", "primvars:displayColor"}
        and operation.get("mesh_conversions", 0) > 0
    ]
    transforms = [operation for operation in operations if operation.get("operation") == "xformOp:translate"]
    require(geometry_rebuilds, f"no geometry edit caused a mesh rebuild: {path}")
    require(transforms and transforms[0].get("mesh_conversions") == 0, f"transform edit rebuilt mesh data: {path}")
    return [f"geometry {path.name}: geometry edit rebuilt mesh data while transform stayed sparse"]


def check_grid_transition(path: Path) -> list[str]:
    report = read_json(path)
    grid = report.get("incident_grid", {})
    require(grid.get("structural_rebuilds", 0) > 0, f"grid structural transition was not observed: {path}")
    return [f"grid {path.name}: structural transition recorded"]


def check_shared_texture(path: Path) -> list[str]:
    report = read_json(path)
    expected = report.get("expected_texture_decode_calls")
    initial = report.get("initial_texture_decode_calls", 0)
    live = report.get("live_texture_decode_calls", 0)
    require(expected == 1, f"shared texture decode bound is not the fixture bound of one: {path}")
    require(initial == expected and live == expected, f"shared texture exceeded or missed its decode bound: {path}")
    require(report.get("live_cleanup_passes", 0) > 0, f"shared texture cleanup was not observed: {path}")
    return [f"texture {path.name}: exact one-decode bound and cleanup recorded"]


def check_c2_comparison(path: Path, max_regression_percent: float | None) -> list[str]:
    report = read_json(path)
    require(report.get("schema") == "usdhub.m10.c2.representative-comparison.v1", f"C2 comparison schema mismatch: {path}")
    require(report.get("passed") is True, f"C2 comparison failed: {path}")
    require(report.get("fixture") == "assets/external/Kitchen_set.usdz", f"C2 comparison fixture mismatch: {path}")
    require(report.get("resolution") == {"width": 1920, "height": 1080}, f"C2 comparison resolution mismatch: {path}")
    matrix = report.get("matrix", {})
    require(matrix.get("baseline_cases") == 16 and matrix.get("candidate_cases") == 16, f"C2 matrix case count mismatch: {path}")
    require(
        matrix.get("baseline_cadence_samples") == 3 and matrix.get("candidate_cadence_samples") == 3,
        f"C2 cadence count mismatch: {path}",
    )
    require(matrix.get("requested_effective_state_matches") is True, f"C2 requested/effective state mismatch: {path}")
    gate = report.get("regression_gate", {})
    observed = number(gate.get("observed_max_regression_percent"), "observed_max_regression_percent")
    require(gate.get("passed") is True, f"C2 regression gate failed: {path}")
    if max_regression_percent is not None:
        require(observed <= max_regression_percent, f"C2 regression exceeds configured limit: {path}")
    return [f"C2 {path.name}: Kitchen baseline/candidate matrix and timing comparison passed ({observed:.2f}% max)"]


def check_c3_matrix(path: Path) -> list[str]:
    report = read_json(path)
    require(report.get("schema") == "usdhub.m10.c3.load-edit-matrix.v1", f"C3 matrix schema mismatch: {path}")
    require(report.get("passed") is True, f"C3 matrix failed: {path}")
    initial_rows = report.get("initial_load_rows", [])
    live_rows = report.get("live_edit_rows", [])
    required_initial = {"small", "representative", "dense", "repeated_geometry", "PointInstancer"}
    required_live = {"transform", "visibility", "material", "geometry", "subtree", "full_fallback"}
    require({row.get("row") for row in initial_rows} == required_initial, f"C3 initial row coverage mismatch: {path}")
    require({row.get("row") for row in live_rows} == required_live, f"C3 live row coverage mismatch: {path}")
    for row in [*initial_rows, *live_rows]:
        require(row.get("status") == "passed", f"C3 row failed: {row.get('row')}")
    point_instancer = next(row for row in initial_rows if row.get("row") == "PointInstancer")
    require(point_instancer.get("asset_counts", {}).get("logical_instances", 0) > 0, f"C3 PointInstancer count missing: {path}")
    geometry = next(row for row in live_rows if row.get("row") == "geometry")
    require(geometry.get("conversion_counts", {}).get("mesh_conversions", 0) > 0, f"C3 geometry conversion missing: {path}")
    for label in ("transform", "visibility", "material"):
        row = next(row for row in live_rows if row.get("row") == label)
        require(row.get("conversion_counts", {}).get("mesh_conversions") == 0, f"C3 {label} unexpectedly converted mesh data: {path}")
    fallback = next(row for row in live_rows if row.get("row") == "full_fallback")
    require(fallback.get("reconcile_counts", {}).get("fallback_extractions", 0) > 0, f"C3 full fallback missing: {path}")
    return [f"C3 {path.name}: five initial-load and six live-edit rows passed"]


def check_persistent_soak(path: Path) -> list[str]:
    report = read_json(path)
    require(report.get("schema") == "usdhub.m10.c4.memory-soak.v2", f"C4 soak schema mismatch: {path}")
    require(report.get("exit_code") == 0, f"C4 persistent runtime failed: {path}")
    require(report.get("cycles", 0) >= 12, f"C4 cycle count is below the contract: {path}")
    require(report.get("rss_high_water_kb", 0) > 0, f"C4 RSS high-water missing: {path}")
    runtime = report.get("runtime", {})
    require(runtime.get("persistent_app") is True, f"C4 runtime was not persistent: {path}")
    require(runtime.get("cycle_count") == report.get("cycles"), f"C4 runtime/report cycle mismatch: {path}")
    require(len(runtime.get("samples", [])) == report.get("cycles"), f"C4 per-cycle samples missing: {path}")
    required_metrics = {
        "mesh_assets",
        "material_assets",
        "image_assets",
        "projection_cache_meshes",
        "projection_cache_sources",
        "material_cache_entries",
        "texture_cache_entries",
    }
    bounds = runtime.get("bounds", [])
    require({bound.get("metric") for bound in bounds} == required_metrics, f"C4 bound coverage mismatch: {path}")
    require(all(bound.get("bounded") is True for bound in bounds), f"C4 cache/asset plateau failed: {path}")
    require(max(sample.get("resize_generation", 0) for sample in runtime["samples"]) >= report["cycles"], f"C4 resize generations missing: {path}")
    require(max(sample.get("point_instancer_full_projects", 0) for sample in runtime["samples"]) > 0, f"C4 PointInstancer reprojection missing: {path}")
    return [f"C4 {path.name}: one persistent runtime, {report['cycles']} cycles, bounded assets/caches and RSS recorded"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=str(ROOT), help="backend checkout root")
    parser.add_argument(
        "--min-renderer-fps",
        type=float,
        default=float(os.environ.get("USDHUB_M10_MIN_RENDERER_FPS", "0")),
        help="optional absolute renderer FPS floor; default disables the floor",
    )
    parser.add_argument(
        "--max-regression-percent",
        type=float,
        default=os.environ.get("USDHUB_M10_MAX_REGRESSION_PERCENT"),
        help="optional C2 relative regression limit; default disables the limit",
    )
    args = parser.parse_args()
    root = Path(args.root).resolve()
    require(args.min_renderer_fps >= 0, "--min-renderer-fps must be non-negative")
    if args.max_regression_percent is not None:
        args.max_regression_percent = float(args.max_regression_percent)
        require(args.max_regression_percent >= 0, "--max-regression-percent must be non-negative")

    reports = root / "target" / "benchmark"
    messages: list[str] = []
    messages.extend(check_c2_comparison(reports / "m10-c2-kitchen-comparison.json", args.max_regression_percent))
    messages.extend(check_c3_matrix(reports / "m10-c3-load-edit-matrix.json"))
    for resolution in ("1280x720", "1920x1080", "2560x1440"):
        messages.extend(check_matrix(reports / f"m10-c2-{resolution}.json", args.min_renderer_fps))
    messages.extend(check_idle_report(reports / "m10-c2-s1-1920x1080.json"))
    messages.extend(check_grid_transition(reports / "m9-final-caa26d7-f6289b9" / "s6.json"))
    messages.extend(check_edit_report(reports / "m9-final-caa26d7-f6289b9" / "s10.json"))
    messages.extend(check_recovery_report(reports / "m9-final-caa26d7-f6289b9" / "s17.json"))
    messages.extend(check_geometry_edit(root / "target" / "m5-c4-live-mesh-patch.json"))
    messages.extend(check_shared_texture(root / "target" / "m6-c5-shared-material.json"))
    messages.extend(check_persistent_soak(reports / "m10-c4-memory-soak.json"))
    print("M10-C5 performance regression checks passed")
    for message in messages:
        print(f"  - {message}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as error:
        print(f"M10-C5 performance regression check failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
