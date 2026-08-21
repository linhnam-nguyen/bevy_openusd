#!/usr/bin/env python3
"""Build a comparable M10-C2 Kitchen_set baseline/candidate report."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def percent_delta(baseline: float | None, candidate: float | None) -> float | None:
    if baseline is None or candidate is None or baseline == 0:
        return None
    return (candidate - baseline) / baseline * 100.0


def validate_pair(
    baseline_matrix: dict[str, Any],
    candidate_matrix: dict[str, Any],
    baseline_s1: dict[str, Any],
    candidate_s1: dict[str, Any],
) -> None:
    for label, report in (
        ("baseline matrix", baseline_matrix),
        ("candidate matrix", candidate_matrix),
    ):
        require(report.get("passed") is True, f"{label} did not pass")
        require(len(report.get("cases", [])) == 16, f"{label} does not contain 16 cases")
        require(
            len(report.get("cadence_samples", [])) == 3,
            f"{label} does not contain three cadence samples",
        )
        identity = report.get("identity", {})
        require(identity.get("scene_label") == "Kitchen_set.usdz", f"{label} is not Kitchen_set")
        require(identity.get("width") == 1920 and identity.get("height") == 1080, f"{label} is not 1920x1080")
        require(report.get("warmup_frames") == 5, f"{label} warmup differs from the contract")
        require(report.get("measured_frames") == 30, f"{label} sample count differs from the contract")
        for case in report["cases"]:
            require(case.get("accepted") is True, f"{label} has an unapplied renderer case")
            require(case.get("configuration_matches") is True, f"{label} has a configuration mismatch")
        for sample in report["cadence_samples"]:
            summary = sample.get("summary", {})
            require(
                summary.get("effective_renderer_target_fps") == sample.get("requested_fps"),
                f"{label} has a cadence authority mismatch",
            )

    base_identity = baseline_matrix["identity"]
    candidate_identity = candidate_matrix["identity"]
    for field in ("scene_label", "scene_hash", "width", "height", "backend", "gpu_adapter"):
        require(
            base_identity.get(field) == candidate_identity.get(field),
            f"C2 baseline/candidate {field} differs",
        )

    for label, report in (("baseline S1", baseline_s1), ("candidate S1", candidate_s1)):
        identity = report.get("identity", {})
        require(identity.get("scene_label") == "assets/external/Kitchen_set.usdz", f"{label} is not Kitchen_set")
        require(identity.get("width") == 1920 and identity.get("height") == 1080, f"{label} is not 1920x1080")
        require(identity.get("build_profile") == "release", f"{label} is not release profile")
        require(report.get("configuration_matches") is True, f"{label} configuration mismatch")
        require(report.get("steady_state_matches") is True, f"{label} steady-state mismatch")

    s1_identity = (baseline_s1["identity"], candidate_s1["identity"])
    require(s1_identity[0].get("scene_hash") == s1_identity[1].get("scene_hash"), "C2 S1 fixture hash differs")
    require(s1_identity[0].get("backend") == s1_identity[1].get("backend"), "C2 S1 backend differs")
    require(s1_identity[0].get("gpu_adapter") == s1_identity[1].get("gpu_adapter"), "C2 S1 GPU differs")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-matrix", type=Path, required=True)
    parser.add_argument("--candidate-matrix", type=Path, required=True)
    parser.add_argument("--baseline-s1", type=Path, required=True)
    parser.add_argument("--candidate-s1", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--max-regression-percent",
        type=float,
        default=None,
        help="optional upper bound for median/p95 CPU time regression and FPS loss",
    )
    args = parser.parse_args()
    if args.max_regression_percent is not None:
        require(args.max_regression_percent >= 0, "max regression must be non-negative")

    baseline_matrix = read_json(args.baseline_matrix)
    candidate_matrix = read_json(args.candidate_matrix)
    baseline_s1 = read_json(args.baseline_s1)
    candidate_s1 = read_json(args.candidate_s1)
    validate_pair(baseline_matrix, candidate_matrix, baseline_s1, candidate_s1)

    base_timing = baseline_s1["timing"]
    candidate_timing = candidate_s1["timing"]
    metrics = {
        "median_frame_ms": {
            "baseline": base_timing.get("median_frame_ms"),
            "candidate": candidate_timing.get("median_frame_ms"),
            "delta_percent": percent_delta(
                base_timing.get("median_frame_ms"), candidate_timing.get("median_frame_ms")
            ),
        },
        "p95_frame_ms": {
            "baseline": base_timing.get("p95_frame_ms"),
            "candidate": candidate_timing.get("p95_frame_ms"),
            "delta_percent": percent_delta(
                base_timing.get("p95_frame_ms"), candidate_timing.get("p95_frame_ms")
            ),
        },
        "actual_renderer_fps": {
            "baseline": base_timing.get("actual_renderer_fps"),
            "candidate": candidate_timing.get("actual_renderer_fps"),
            "delta_percent": percent_delta(
                base_timing.get("actual_renderer_fps"), candidate_timing.get("actual_renderer_fps")
            ),
        },
        "gpu_median_frame_ms": {
            "baseline": base_timing.get("gpu_median_frame_ms"),
            "candidate": candidate_timing.get("gpu_median_frame_ms"),
            "delta_percent": percent_delta(
                base_timing.get("gpu_median_frame_ms"), candidate_timing.get("gpu_median_frame_ms")
            ),
        },
        "gpu_p95_frame_ms": {
            "baseline": base_timing.get("gpu_p95_frame_ms"),
            "candidate": candidate_timing.get("gpu_p95_frame_ms"),
            "delta_percent": percent_delta(
                base_timing.get("gpu_p95_frame_ms"), candidate_timing.get("gpu_p95_frame_ms")
            ),
        },
    }
    regression_metrics = [metrics[name]["delta_percent"] for name in ("median_frame_ms", "p95_frame_ms")]
    fps_loss = -(metrics["actual_renderer_fps"]["delta_percent"] or 0.0)
    observed_regression = max([value for value in regression_metrics if value is not None] + [fps_loss])
    threshold_passed = args.max_regression_percent is None or observed_regression <= args.max_regression_percent
    require(threshold_passed, f"C2 regression {observed_regression:.2f}% exceeds configured limit")

    result = {
        "schema": "usdhub.m10.c2.representative-comparison.v1",
        "checkpoint": "M10-C2+",
        "camera_profile": "default-launch-camera",
        "fixture": "assets/external/Kitchen_set.usdz",
        "resolution": {"width": 1920, "height": 1080},
        "warmup_frames": 5,
        "measured_frames": 30,
        "baseline": {
            "git_sha": baseline_matrix["identity"]["git_sha"],
            "matrix": str(args.baseline_matrix),
            "s1": str(args.baseline_s1),
        },
        "candidate": {
            "git_sha": candidate_matrix["identity"]["git_sha"],
            "matrix": str(args.candidate_matrix),
            "s1": str(args.candidate_s1),
        },
        "matrix": {
            "baseline_cases": len(baseline_matrix["cases"]),
            "candidate_cases": len(candidate_matrix["cases"]),
            "baseline_cadence_samples": len(baseline_matrix["cadence_samples"]),
            "candidate_cadence_samples": len(candidate_matrix["cadence_samples"]),
            "requested_effective_state_matches": True,
        },
        "timing_comparison": metrics,
        "regression_gate": {
            "max_regression_percent": args.max_regression_percent,
            "observed_max_regression_percent": observed_regression,
            "passed": threshold_passed,
        },
        "passed": True,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"M10-C2 representative comparison passed: {args.output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError, ValueError) as error:
        raise SystemExit(f"M10-C2 representative comparison failed: {error}") from error
