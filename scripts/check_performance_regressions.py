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
    initial = report.get("initial_texture_decode_calls", 0)
    live = report.get("live_texture_decode_calls", 0)
    require(initial >= 1 and live >= 1, f"shared texture was not decoded in both phases: {path}")
    require(report.get("live_cleanup_passes", 0) > 0, f"shared texture cleanup was not observed: {path}")
    return [f"texture {path.name}: repeated phase lookups and cleanup recorded"]


def check_soak(path: Path) -> list[str]:
    report = read_json(path)
    workloads = report.get("workloads", [])
    require(len(workloads) == 6, f"memory soak workload count is not six: {path}")
    for workload in workloads:
        require(workload.get("exit_code") == 0, f"memory soak workload failed: {workload.get('label')}")
        require(workload.get("rss_high_water_kb", 0) > 0, f"memory high-water missing: {workload.get('label')}")
    return [f"soak {path.name}: six workloads exited and reported RSS high-water"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=str(ROOT), help="backend checkout root")
    parser.add_argument(
        "--min-renderer-fps",
        type=float,
        default=float(os.environ.get("USDHUB_M10_MIN_RENDERER_FPS", "0")),
        help="optional absolute renderer FPS floor; default disables the floor",
    )
    args = parser.parse_args()
    root = Path(args.root).resolve()
    require(args.min_renderer_fps >= 0, "--min-renderer-fps must be non-negative")

    reports = root / "target" / "benchmark"
    messages: list[str] = []
    for resolution in ("1280x720", "1920x1080", "2560x1440"):
        messages.extend(check_matrix(reports / f"m10-c2-{resolution}.json", args.min_renderer_fps))
    messages.extend(check_idle_report(reports / "m10-c2-s1-1920x1080.json"))
    messages.extend(check_grid_transition(reports / "m9-final-caa26d7-f6289b9" / "s6.json"))
    messages.extend(check_edit_report(reports / "m9-final-caa26d7-f6289b9" / "s10.json"))
    messages.extend(check_recovery_report(reports / "m9-final-caa26d7-f6289b9" / "s17.json"))
    messages.extend(check_geometry_edit(root / "target" / "m5-c4-live-mesh-patch.json"))
    messages.extend(check_shared_texture(root / "target" / "m6-c5-shared-material.json"))
    messages.extend(check_soak(reports / "m10-c4-memory-soak.json"))
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
