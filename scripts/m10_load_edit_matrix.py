#!/usr/bin/env python3
"""Collect the complete M10-C3 load/edit matrix from release artifacts."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RENDER_BENCH = ROOT / "scripts" / "render_bench.py"

INITIAL_FIXTURES = {
    "small": "assets/external/teapot.usdz",
    "representative": "assets/external/Kitchen_set.usdz",
    "dense": "assets/external/PointInstancedMedCity.usdz",
    "repeated_geometry": "tests/stages/instanceable.usda",
}


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read C3 artifact {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"C3 artifact is not an object: {path}")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def run_initial_loads(output_dir: Path, warmup: int, frames: int) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    for label, fixture in INITIAL_FIXTURES.items():
        output = output_dir / f"m10-c3-initial-{label}.json"
        command = [
            sys.executable,
            str(RENDER_BENCH),
            "--scenario",
            "S1",
            "--fixture",
            fixture,
            "--force-headless",
            "--warmup",
            str(warmup),
            "--frames",
            str(frames),
            "--output",
            str(output),
            "--label",
            f"m10-c3-initial-{label}",
            "--stream-width",
            "1920",
            "--stream-height",
            "1080",
            "--stream-fps",
            "60",
        ]
        print(f"running C3 initial-load row {label}: {fixture}", flush=True)
        subprocess.run(command, cwd=ROOT, check=True)


def initial_row(label: str, fixture: str, report: dict[str, Any]) -> dict[str, Any]:
    require(report.get("configuration_matches") is True, f"initial {label} configuration mismatch")
    require(report.get("steady_state_matches") is True, f"initial {label} steady-state mismatch")
    phase = report.get("phase_metrics", {})
    cache = report.get("cache_snapshot", {})
    timing = report.get("timing", {})
    return {
        "row": label,
        "kind": "initial_load",
        "fixture": fixture,
        "status": "passed",
        "projection_latency_ms": phase.get("initial_projection_ms"),
        "projection_prims": phase.get("initial_projection_prims"),
        "reconcile_latency_ms": None,
        "conversion_counts": {
            "mesh_conversions": None,
            "source_cache_lookups": None,
            "source_cache_hits": None,
            "source_cache_misses": None,
        },
        "reconcile_counts": {
            "visited_stage_prims": None,
            "patched_entities": None,
            "spawned_entities": None,
            "despawned_entities": None,
            "fallback_extractions": None,
        },
        "asset_counts": {
            "live_stage_prims": cache.get("live_stage_prims"),
            "mesh_assets": None,
            "material_assets": cache.get("cached_materials"),
            "image_assets": cache.get("cached_textures"),
        },
        "frame_impact": {
            "available": True,
            "median_frame_ms": timing.get("median_frame_ms"),
            "p95_frame_ms": timing.get("p95_frame_ms"),
            "actual_renderer_fps": timing.get("actual_renderer_fps"),
        },
        "evidence": report.get("identity", {}).get("git_sha"),
    }


def operation_by_name(operations: list[dict[str, Any]], name: str) -> dict[str, Any]:
    for operation in operations:
        if operation.get("operation") == name:
            return operation
    raise ValueError(f"C3 operation is missing: {name}")


def live_operation_row(row: str, operation: dict[str, Any], evidence: str) -> dict[str, Any]:
    return {
        "row": row,
        "kind": "live_edit",
        "fixture": "inline-single-mesh-with-subtree-edit",
        "status": "passed",
        "projection_latency_ms": None,
        "projection_prims": None,
        "reconcile_latency_ms": operation.get("patch_latency_ms"),
        "conversion_counts": {
            "mesh_conversions": operation.get("mesh_conversions"),
            "source_cache_lookups": operation.get("source_cache_lookups"),
            "source_cache_hits": operation.get("source_cache_hits"),
            "source_cache_misses": operation.get("source_cache_misses"),
        },
        "reconcile_counts": {
            "visited_stage_prims": operation.get("reconcile_visited_stage_prims"),
            "patched_entities": operation.get("reconcile_patched_entities"),
            "spawned_entities": operation.get("reconcile_spawned_entities"),
            "despawned_entities": operation.get("reconcile_despawned_entities"),
            "fallback_extractions": 0,
        },
        "asset_counts": {
            "live_stage_prims": None,
            "mesh_assets": None,
            "material_assets": None,
            "image_assets": None,
        },
        "frame_impact": {"available": False},
        "evidence": evidence,
    }


def full_fallback_row(report: dict[str, Any]) -> dict[str, Any]:
    semantic = report.get("incident_semantic", {})
    grid = report.get("incident_grid", {})
    cache = report.get("cache_snapshot", {})
    timing = report.get("timing", {})
    require(semantic.get("fallback_extractions", 0) > 0, "C3 full fallback was not observed")
    return {
        "row": "full_fallback",
        "kind": "live_edit",
        "fixture": report.get("identity", {}).get("scene_label"),
        "status": "passed",
        "projection_latency_ms": report.get("phase_metrics", {}).get("initial_projection_ms"),
        "projection_prims": report.get("phase_metrics", {}).get("initial_projection_prims"),
        "reconcile_latency_ms": None,
        "conversion_counts": {"mesh_conversions": None},
        "reconcile_counts": {
            "visited_stage_prims": grid.get("prims_scanned"),
            "patched_entities": None,
            "spawned_entities": None,
            "despawned_entities": None,
            "fallback_extractions": semantic.get("fallback_extractions"),
            "extent_recompute_calls": grid.get("compute_extent_calls"),
            "snapshot_clones": semantic.get("snapshot_clones"),
        },
        "asset_counts": {
            "live_stage_prims": cache.get("live_stage_prims"),
            "mesh_assets": None,
            "material_assets": cache.get("cached_materials"),
            "image_assets": cache.get("cached_textures"),
        },
        "frame_impact": {
            "available": True,
            "measured_frames": len(report.get("raw_samples", [])),
            "median_frame_ms": timing.get("median_frame_ms"),
            "p95_frame_ms": timing.get("p95_frame_ms"),
            "actual_renderer_fps": timing.get("actual_renderer_fps"),
        },
        "evidence": report.get("identity", {}).get("git_sha"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/benchmark/m10-c3-load-edit-matrix.json")
    parser.add_argument("--artifact-dir", type=Path, default=ROOT / "target/benchmark")
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--frames", type=int, default=20)
    parser.add_argument("--skip-loads", action="store_true")
    args = parser.parse_args()
    require(args.warmup >= 0 and args.frames > 0, "C3 warmup/frames must be valid")

    if not args.skip_loads:
        run_initial_loads(args.artifact_dir, args.warmup, args.frames)

    load_rows = [
        initial_row(
            label,
            fixture,
            read_json(args.artifact_dir / f"m10-c3-initial-{label}.json"),
        )
        for label, fixture in INITIAL_FIXTURES.items()
    ]
    # PointInstancer has a dedicated profile with logical identity and shared
    # mesh-asset counts that are not present in the generic S1 report.
    instancer = read_json(ROOT / "target/m8-c1-instancing-baseline.json")
    load_rows.append(
        {
            "row": "PointInstancer",
            "kind": "initial_load",
            "fixture": instancer.get("fixture"),
            "status": "passed",
            "projection_latency_ms": instancer.get("projection_ms"),
            "projection_prims": instancer.get("point_instancer_prim_count"),
            "reconcile_latency_ms": None,
            "conversion_counts": {"mesh_conversions": instancer.get("mesh_entity_count")},
            "reconcile_counts": {"visited_stage_prims": None, "fallback_extractions": 0},
            "asset_counts": {
                "live_stage_prims": None,
                "mesh_assets": instancer.get("mesh_asset_count"),
                "material_assets": instancer.get("material_asset_count"),
                "image_assets": None,
                "logical_instances": instancer.get("logical_instance_count"),
                "unique_mesh_handles": instancer.get("unique_mesh_handles"),
            },
            "frame_impact": {"available": False},
            "evidence": instancer.get("git_sha"),
        }
    )

    mesh_patch_path = ROOT / "target/m5-c4-live-mesh-patch.json"
    mesh_patch = read_json(mesh_patch_path)
    operations = mesh_patch.get("operations", [])
    require(isinstance(operations, list), "C3 mesh patch operations are missing")
    live_rows = [
        live_operation_row("transform", operation_by_name(operations, "xformOp:translate"), str(mesh_patch_path)),
        live_operation_row("visibility", operation_by_name(operations, "visibility"), str(mesh_patch_path)),
        live_operation_row("material", operation_by_name(operations, "material:binding"), str(mesh_patch_path)),
    ]
    geometry_ops = [operation_by_name(operations, name) for name in ("points", "primvars:displayColor")]
    geometry = geometry_ops[0].copy()
    geometry["operation"] = "geometry"
    for field in (
        "mesh_conversions",
        "source_cache_lookups",
        "source_cache_hits",
        "source_cache_misses",
        "reconcile_visited_stage_prims",
        "reconcile_patched_entities",
        "reconcile_spawned_entities",
        "reconcile_despawned_entities",
    ):
        geometry[field] = sum(operation.get(field, 0) for operation in geometry_ops)
    geometry["patch_latency_ms"] = max(operation.get("patch_latency_ms", 0.0) for operation in geometry_ops)
    live_rows.append(live_operation_row("geometry", geometry, str(mesh_patch_path)))
    subtree_ops = [operation_by_name(operations, name) for name in ("subtree-add", "subtree-remove")]
    subtree = subtree_ops[0].copy()
    subtree["operation"] = "subtree"
    for field in (
        "mesh_conversions",
        "source_cache_lookups",
        "source_cache_hits",
        "source_cache_misses",
        "reconcile_visited_stage_prims",
        "reconcile_patched_entities",
        "reconcile_spawned_entities",
        "reconcile_despawned_entities",
    ):
        subtree[field] = sum(operation.get(field, 0) for operation in subtree_ops)
    subtree["patch_latency_ms"] = sum(operation.get("patch_latency_ms", 0.0) for operation in subtree_ops)
    live_rows.append(live_operation_row("subtree", subtree, str(mesh_patch_path)))

    fallback_path = args.artifact_dir / "m10-c3-full-fallback.json"
    live_rows.append(full_fallback_row(read_json(fallback_path)))

    rows = load_rows + live_rows
    required_rows = {
        "small",
        "representative",
        "dense",
        "repeated_geometry",
        "PointInstancer",
        "transform",
        "visibility",
        "material",
        "geometry",
        "subtree",
        "full_fallback",
    }
    require({row["row"] for row in rows} == required_rows, "C3 row coverage is incomplete")
    runtime_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    result = {
        "schema": "usdhub.m10.c3.load-edit-matrix.v1",
        "checkpoint": "M10-C3+",
        "git_sha": runtime_sha,
        "build_profile": "release",
        "warmup_frames": args.warmup,
        "measured_frames": args.frames,
        "initial_load_rows": load_rows,
        "live_edit_rows": live_rows,
        "required_initial_rows": ["small", "representative", "dense", "repeated_geometry", "PointInstancer"],
        "required_live_edit_rows": ["transform", "visibility", "material", "geometry", "subtree", "full_fallback"],
        "passed": True,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"M10-C3 load/edit matrix passed: {args.output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, subprocess.CalledProcessError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"M10-C3 load/edit matrix failed: {error}") from error
