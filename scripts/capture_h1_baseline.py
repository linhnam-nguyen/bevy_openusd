#!/usr/bin/env python3
"""Capture the H1 pre-change Kitchen native and headless WebRTC baseline."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = "assets/external/Kitchen_set.usdz"


def git_sha() -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()


def capture(scenario: str, output: Path, warmup: int, frames: int) -> None:
    command = [
        sys.executable,
        str(ROOT / "scripts" / "render_bench.py"),
        "--scenario",
        scenario,
        "--fixture",
        FIXTURE,
        "--force-headless",
        "--warmup",
        str(warmup),
        "--frames",
        str(frames),
        "--output",
        str(output),
        "--label",
        "h1-c2-baseline",
    ]
    subprocess.run(command, cwd=ROOT, check=True)


def validate_report(path: Path, scenario: str) -> dict:
    report = json.loads(path.read_text(encoding="utf-8"))
    identity = report.get("identity", {})
    timing = report.get("timing", {})
    if identity.get("scene_label") != FIXTURE:
        raise ValueError(f"{path}: fixture identity is not Kitchen_set.usdz")
    if identity.get("scenario_code") != scenario:
        raise ValueError(f"{path}: scenario identity is not {scenario}")
    if report.get("configuration_matches") is not True:
        raise ValueError(f"{path}: effective configuration did not match")
    if report.get("steady_state_matches") is not True:
        raise ValueError(f"{path}: steady-state invariants did not match")
    for field in ("median_frame_ms", "p95_frame_ms", "actual_renderer_fps"):
        if not isinstance(timing.get(field), (int, float)):
            raise ValueError(f"{path}: missing numeric timing.{field}")
    return {
        "scenario": scenario,
        "report": str(path),
        "median_frame_ms": timing["median_frame_ms"],
        "p95_frame_ms": timing["p95_frame_ms"],
        "actual_renderer_fps": timing["actual_renderer_fps"],
        "incident_semantic": report.get("incident_semantic", {}),
        "incident_grid": report.get("incident_grid", {}),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / "target" / "benchmark" / "h1-c2-baseline",
    )
    parser.add_argument("--warmup", type=int, default=30)
    parser.add_argument("--frames", type=int, default=120)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)

    native_path = args.output_dir / "native-s1.json"
    webrtc_path = args.output_dir / "webrtc-s11.json"
    capture("S1", native_path, args.warmup, args.frames)
    capture("S11", webrtc_path, args.warmup, args.frames)
    packet = {
        "schema": "usdhub.h1.c2.postupdate-baseline.v1",
        "checkpoint": "H1-C2",
        "source_sha": git_sha(),
        "fixture": FIXTURE,
        "settings": {
            "build_profile": "release",
            "width": 1920,
            "height": 1080,
            "requested_fps": 60,
            "warmup_frames": args.warmup,
            "measured_frames": args.frames,
        },
        "cases": [
            validate_report(native_path, "S1"),
            validate_report(webrtc_path, "S11"),
        ],
        "client_evidence": {
            "status": "not_applicable_to_C2",
            "note": "Real-client S12-S18 evidence is captured by H1-C7.",
        },
    }
    packet_path = args.output_dir / "baseline.json"
    packet_path.write_text(json.dumps(packet, indent=2) + "\n", encoding="utf-8")
    print(f"H1-C2 baseline passed: {packet_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
