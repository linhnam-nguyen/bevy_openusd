#!/usr/bin/env python3
"""
render_bench_compare.py — Compare benchmark JSON reports between runs/checkpoints.

Usage:
    python3 scripts/render_bench_compare.py target/benchmark/baseline/s1.json target/benchmark/optimized/s1.json
    python3 scripts/render_bench_compare.py --dir-a target/benchmark/baseline --dir-b target/benchmark/optimized
"""

import argparse
import json
import os
import sys
from pathlib import Path

REQUIRED_TOP_LEVEL_FIELDS = [
    "schema_version", "identity", "requested_configuration", "effective_configuration",
    "configuration_matches", "expected_steady_state", "observed_steady_state",
    "steady_state_matches", "timing", "incident_grid", "incident_semantic",
    "webrtc_metrics", "isolation_metrics", "phase_metrics", "cache_snapshot", "raw_samples",
]
REQUIRED_IDENTITY_FIELDS = [
    "schema_version", "checkpoint_id", "git_sha", "glacial_sha", "scenario_code",
    "scene_label", "scene_path", "build_profile", "os", "backend", "gpu_adapter",
    "width", "height", "requested_fps",
]
REQUIRED_TIMING_FIELDS = [
    "warmup_frames", "measured_frames", "median_frame_ms", "p95_frame_ms", "min_frame_ms",
    "max_frame_ms", "actual_renderer_fps", "avg_fps", "p95_fps_equivalent", "wall_median_ms",
    "wall_p95_ms", "gpu_median_frame_ms", "gpu_p95_frame_ms",
]
REQUIRED_GRID_FIELDS = [
    "compute_extent_calls", "prims_scanned", "sync_calls", "host_writes", "visible_writes",
    "ground_y_writes", "coverage_radius_writes", "value_changes", "changed_observations",
    "update_alpha_calls", "lines_rebuilt", "dots_rebuilt", "structural_rebuilds",
    "vertices_generated", "indices_generated",
]
REQUIRED_SEM_FIELDS = [
    "sync_calls", "idle_skips", "snapshot_clones", "initial_extractions",
    "initial_extraction_failures", "fallback_extractions", "subtree_extractions",
    "worker_submissions", "worker_submission_failures", "recovery_checkpoints", "recovery_successes",
]
REQUIRED_WEBRTC_FIELDS = [
    "remote_commands_drained", "remote_inputs_applied", "authoritative_events_published",
    "captured_frames", "frame_queue_drops",
]
REQUIRED_ISOLATION_FIELDS = [
    "sync_db_auth_waits_in_bevy", "query_saturations", "query_requests", "query_results",
    "query_failures", "query_high_water", "query_median_latency_ms", "query_p95_latency_ms",
    "auth_validation_bursts", "auth_lookup_count", "auth_snapshot_hits", "auth_validations",
    "auth_failures", "auth_high_water",
]
REQUIRED_CACHE_FIELDS = [
    "live_stage_prims", "live_stage_animated_prims", "cached_materials", "cached_textures",
    "material_hits", "material_misses", "texture_hits", "texture_misses",
]
REQUIRED_PHASE_FIELDS = [
    "initial_projection_ms", "initial_projection_prims", "stage_traversal_ms", "mesh_generation_ms",
    "primvar_expansion_ms", "normal_generation_ms", "material_resolve_ms",
]
REQUIRED_SECTIONS = [
    "identity", "timing", "incident_grid", "incident_semantic", "webrtc_metrics",
    "isolation_metrics", "cache_snapshot", "phase_metrics",
]

def load_report(path: str) -> dict:
    if not os.path.exists(path):
        raise FileNotFoundError(f"Report file does not exist: {path}")
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
    if "schema_version" not in data:
        raise ValueError(f"Invalid benchmark report schema (missing schema_version): {path}")
    return data

def validate_required_fields(data: dict, file_label: str):
    for field in REQUIRED_TOP_LEVEL_FIELDS:
        if field not in data:
            raise ValueError(f"{file_label} missing required top-level field: {field}")
    if data["schema_version"] != 1:
        raise ValueError(f"{file_label} has unsupported schema_version: {data['schema_version']}")

    ident = data["identity"]
    timing = data["timing"]
    grid = data["incident_grid"]
    sem = data["incident_semantic"]
    webrtc = data["webrtc_metrics"]
    isolation = data["isolation_metrics"]
    cache = data["cache_snapshot"]
    phase = data["phase_metrics"]

    for f in REQUIRED_IDENTITY_FIELDS:
        if f not in ident:
            raise ValueError(f"{file_label} missing required identity field: {f}")
    for f in REQUIRED_TIMING_FIELDS:
        if f not in timing:
            raise ValueError(f"{file_label} missing required timing field: {f}")
    for f in REQUIRED_GRID_FIELDS:
        if f not in grid:
            raise ValueError(f"{file_label} missing required incident_grid field: {f}")
    for f in REQUIRED_SEM_FIELDS:
        if f not in sem:
            raise ValueError(f"{file_label} missing required incident_semantic field: {f}")
    for section, fields, value in (
        ("webrtc_metrics", REQUIRED_WEBRTC_FIELDS, webrtc),
        ("isolation_metrics", REQUIRED_ISOLATION_FIELDS, isolation),
        ("cache_snapshot", REQUIRED_CACHE_FIELDS, cache),
        ("phase_metrics", REQUIRED_PHASE_FIELDS, phase),
    ):
        for field in fields:
            if field not in value:
                raise ValueError(f"{file_label} missing required {section} field: {field}")

def compare_single(report_a: dict, report_b: dict, label_a: str = "Baseline", label_b: str = "Candidate") -> bool:
    validate_required_fields(report_a, label_a)
    validate_required_fields(report_b, label_b)

    id_a = report_a["identity"]
    id_b = report_b["identity"]

    timing_a = report_a["timing"]
    timing_b = report_b["timing"]

    grid_a = report_a["incident_grid"]
    grid_b = report_b["incident_grid"]

    sem_a = report_a["incident_semantic"]
    sem_b = report_b["incident_semantic"]

    webrtc_a = report_a["webrtc_metrics"]
    webrtc_b = report_b["webrtc_metrics"]

    iso_a = report_a["isolation_metrics"]
    iso_b = report_b["isolation_metrics"]

    cache_a = report_a["cache_snapshot"]
    cache_b = report_b["cache_snapshot"]

    # Strict configuration invariant checks
    if id_a["scenario_code"] != id_b["scenario_code"]:
        raise ValueError(f"Scenario code mismatch: {id_a['scenario_code']} vs {id_b['scenario_code']}")
    if id_a["scene_label"] != id_b["scene_label"]:
        raise ValueError(f"Scene label mismatch: {id_a['scene_label']} vs {id_b['scene_label']}")
    if (id_a["width"], id_a["height"]) != (id_b["width"], id_b["height"]):
        raise ValueError(f"Resolution mismatch: {id_a['width']}x{id_a['height']} vs {id_b['width']}x{id_b['height']}")
    if id_a["build_profile"] != id_b["build_profile"]:
        raise ValueError(f"Build profile mismatch: {id_a['build_profile']} vs {id_b['build_profile']}")
    if id_a["backend"] != id_b["backend"]:
        raise ValueError(f"Renderer backend mismatch: {id_a['backend']} vs {id_b['backend']}")
    if id_a["gpu_adapter"] != id_b["gpu_adapter"]:
        raise ValueError(f"GPU adapter mismatch: {id_a['gpu_adapter']} vs {id_b['gpu_adapter']}")
    if id_a["glacial_sha"] != id_b["glacial_sha"]:
        raise ValueError(f"Glacial SHA mismatch: {id_a['glacial_sha']} vs {id_b['glacial_sha']}")
    if id_a["requested_fps"] != id_b["requested_fps"]:
        raise ValueError(f"Requested FPS mismatch: {id_a['requested_fps']} vs {id_b['requested_fps']}")

    cfg_match_a = report_a["configuration_matches"]
    cfg_match_b = report_b["configuration_matches"]
    cfg_consistent = report_a["requested_configuration"] == report_b["requested_configuration"]

    if not cfg_consistent:
        raise ValueError(f"Requested configuration mismatch between {label_a} and {label_b}")

    med_a = timing_a["median_frame_ms"]
    med_b = timing_b["median_frame_ms"]
    p95_a = timing_a["p95_frame_ms"]
    p95_b = timing_b["p95_frame_ms"]
    fps_a = timing_a["actual_renderer_fps"]
    fps_b = timing_b["actual_renderer_fps"]

    speedup_med = ((med_a - med_b) / med_a * 100.0) if med_a > 0 else 0.0
    speedup_p95 = ((p95_a - p95_b) / p95_a * 100.0) if p95_a > 0 else 0.0
    fps_delta = ((fps_b - fps_a) / fps_a * 100.0) if fps_a > 0 else 0.0

    print(f"\n{'='*80}")
    print(f"BENCHMARK COMPARISON: {id_a.get('scene_label', 'Unknown')} [{id_a.get('scenario_code', 'N/A')}]")
    print(f"A: {label_a} [{id_a.get('checkpoint_id', 'N/A')}] ({id_a.get('git_sha', '')[:8]})")
    print(f"B: {label_b} [{id_b.get('checkpoint_id', 'N/A')}] ({id_b.get('git_sha', '')[:8]})")
    print(f"{'='*80}")

    print(f"{'Metric':<36} | {label_a:<15} | {label_b:<15} | {'Delta / Status'}")
    print(f"{'-'*36}-|-{'-'*15}-|-{'-'*15}-|-{'-'*15}")
    print(f"{'Median CPU Frame (ms)':<36} | {med_a:<15.3f} | {med_b:<15.3f} | {speedup_med:+.1f}%")
    print(f"{'P95 CPU Frame (ms)':<36} | {p95_a:<15.3f} | {p95_b:<15.3f} | {speedup_p95:+.1f}%")
    print(f"{'Actual Bevy Renderer FPS':<36} | {fps_a:<15.1f} | {fps_b:<15.1f} | {fps_delta:+.1f}%")

    print(f"{'Configuration Invariant Matches':<36} | {str(cfg_match_a):<15} | {str(cfg_match_b):<15} | {'OK' if cfg_consistent else 'INCOMPARABLE'}")

    # Grid Incidents (Incident A)
    grid_rebuild_a = grid_a["structural_rebuilds"]
    grid_rebuild_b = grid_b["structural_rebuilds"]
    status_grid = "CLEAN" if grid_rebuild_b == 0 else f"VIOLATION (+{grid_rebuild_b})"
    print(f"{'Grid Structural Rebuilds':<36} | {grid_rebuild_a:<15} | {grid_rebuild_b:<15} | {status_grid}")

    grid_verts_a = grid_a["vertices_generated"]
    grid_verts_b = grid_b["vertices_generated"]
    print(f"{'Grid Vertices Generated':<36} | {grid_verts_a:<15} | {grid_verts_b:<15} | {grid_verts_b - grid_verts_a:+}")

    # Semantic Incidents (Incident B)
    sem_clones_a = sem_a["snapshot_clones"]
    sem_clones_b = sem_b["snapshot_clones"]
    status_sem = "CLEAN" if sem_clones_b == 0 else f"VIOLATION (+{sem_clones_b})"
    print(f"{'Semantic Snapshot Clones':<36} | {sem_clones_a:<15} | {sem_clones_b:<15} | {status_sem}")

    rec_a = sem_a["recovery_checkpoints"]
    rec_b = sem_b["recovery_checkpoints"]
    print(f"{'Recovery Checkpoints':<36} | {rec_a:<15} | {rec_b:<15} | {rec_b - rec_a:+}")

    # WebRTC Metrics
    cmd_a = webrtc_a["remote_commands_drained"]
    cmd_b = webrtc_b["remote_commands_drained"]
    print(f"{'Remote Commands Drained':<36} | {cmd_a:<15} | {cmd_b:<15} | {cmd_b - cmd_a:+}")

    caps_a = webrtc_a["captured_frames"]
    caps_b = webrtc_b["captured_frames"]
    print(f"{'Captured Video Frames':<36} | {caps_a:<15} | {caps_b:<15} | {caps_b - caps_a:+}")

    # Isolation Invariants
    iso_waits_a = iso_a["sync_db_auth_waits_in_bevy"]
    iso_waits_b = iso_b["sync_db_auth_waits_in_bevy"]
    status_iso = "CLEAN" if iso_waits_b == 0 else f"VIOLATION (+{iso_waits_b})"
    print(f"{'Sync DB Auth Waits in Bevy':<36} | {iso_waits_a:<15} | {iso_waits_b:<15} | {status_iso}")

    # Cache Snapshot
    mats_a = cache_a["cached_materials"]
    mats_b = cache_b["cached_materials"]
    print(f"{'Cached Standard Materials':<36} | {mats_a:<15} | {mats_b:<15} | {mats_b - mats_a:+}")
    print(f"{'='*80}\n")
    return cfg_consistent and cfg_match_a and cfg_match_b

def main():
    parser = argparse.ArgumentParser(description="Compare rendering benchmark JSON reports")
    parser.add_argument("file_a", nargs="?", help="Baseline report JSON file")
    parser.add_argument("file_b", nargs="?", help="Candidate report JSON file")
    parser.add_argument("--dir-a", help="Directory of baseline reports")
    parser.add_argument("--dir-b", help="Directory of candidate reports")
    parser.add_argument("--label-a", default="Baseline", help="Label for run A")
    parser.add_argument("--label-b", default="Candidate", help="Label for run B")

    args = parser.parse_args()

    if args.dir_a and args.dir_b:
        dir_a = Path(args.dir_a)
        dir_b = Path(args.dir_b)
        if not dir_a.exists():
            print(f"Error: Baseline directory {dir_a} does not exist", file=sys.stderr)
            sys.exit(1)
        if not dir_b.exists():
            print(f"Error: Candidate directory {dir_b} does not exist", file=sys.stderr)
            sys.exit(1)

        files_a = sorted(dir_a.glob("*.json"))
        if not files_a:
            print(f"Error: No report JSON files found in {dir_a}", file=sys.stderr)
            sys.exit(1)

        all_ok = True
        for path_a in files_a:
            path_b = dir_b / path_a.name
            if not path_b.exists():
                print(f"Error: Missing candidate evidence for {path_a.name} in {dir_b}", file=sys.stderr)
                all_ok = False
                continue
            try:
                rep_a = load_report(str(path_a))
                rep_b = load_report(str(path_b))
                if not compare_single(rep_a, rep_b, args.label_a, args.label_b):
                    all_ok = False
            except Exception as e:
                print(f"Error comparing {path_a.name}: {e}", file=sys.stderr)
                all_ok = False
        sys.exit(0 if all_ok else 1)

    elif args.file_a and args.file_b:
        try:
            rep_a = load_report(args.file_a)
            rep_b = load_report(args.file_b)
            ok = compare_single(rep_a, rep_b, args.label_a, args.label_b)
            sys.exit(0 if ok else 1)
        except Exception as e:
            print(f"Error comparing reports: {e}", file=sys.stderr)
            sys.exit(1)
    else:
        parser.print_help()
        sys.exit(1)

if __name__ == "__main__":
    main()
