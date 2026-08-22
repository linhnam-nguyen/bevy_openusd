#!/usr/bin/env python3
"""Capture and compare the final H1 Kitchen native/WebRTC regression packet."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from capture_h1_baseline import FIXTURE, capture, git_sha, validate_report


ROOT = Path(__file__).resolve().parents[1]
REQUIRED_SEMANTIC_FIELDS = {
    "semantic_extract_ms",
    "render_blob_prepare_ms",
    "total_semantic_postupdate_ms",
    "runtime_delivery_submit_ms",
    "runtime_delivery_worker_ms",
    "recovery_serialize_ms",
    "recovery_submit_ms",
    "recovery_worker_write_ms",
    "semantic_mailbox_pending",
    "semantic_mailbox_high_water",
}


def read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"artifact is not an object: {path}")
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / "target" / "benchmark" / "h1-c7-regression",
    )
    parser.add_argument("--warmup", type=int, default=30)
    parser.add_argument("--frames", type=int, default=120)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)

    native_path = args.output_dir / "native-s1.json"
    webrtc_path = args.output_dir / "webrtc-s11.json"
    capture("S1", native_path, args.warmup, args.frames)
    capture("S11", webrtc_path, args.warmup, args.frames)
    native = validate_report(native_path, "S1")
    webrtc = validate_report(webrtc_path, "S11")
    for case in (native, webrtc):
        missing = REQUIRED_SEMANTIC_FIELDS.difference(case["incident_semantic"])
        if missing:
            raise ValueError(f"{case['report']}: missing H1 telemetry {sorted(missing)}")

    c2_packet_path = ROOT / "target" / "benchmark" / "h1-c2-baseline" / "baseline.json"
    c2_packet = read_json(c2_packet_path)
    c2_cases = {case["scenario"]: case for case in c2_packet.get("cases", [])}
    comparison = []
    for candidate in (native, webrtc):
        baseline = c2_cases.get(candidate["scenario"])
        if baseline is None:
            raise ValueError(f"H1-C2 baseline is missing {candidate['scenario']}")
        comparison.append(
            {
                "scenario": candidate["scenario"],
                "baseline": baseline,
                "candidate": candidate,
                "median_frame_delta_percent": (
                    (candidate["median_frame_ms"] - baseline["median_frame_ms"])
                    / baseline["median_frame_ms"]
                    * 100.0
                    if baseline["median_frame_ms"]
                    else None
                ),
                "p95_frame_delta_percent": (
                    (candidate["p95_frame_ms"] - baseline["p95_frame_ms"])
                    / baseline["p95_frame_ms"]
                    * 100.0
                    if baseline["p95_frame_ms"]
                    else None
                ),
            }
        )

    packet = {
        "schema": "usdhub.h1.c7.runtime-isolation-regression.v1",
        "checkpoint": "H1-C7",
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
        "cases": comparison,
        "real_client_evidence": {
            "status": "inherited_from_M10",
            "scenarios": ["S12", "S13", "S14", "S15", "S16", "S17", "S18"],
            "note": "H1 changes are backend-only; fresh real-client proof remains the frozen M10 packet.",
        },
        "m10_gates": {
            "status": "run_separately",
            "commands": ["make harden", "make bench-render-smoke"],
        },
    }
    output = args.output_dir / "regression.json"
    output.write_text(json.dumps(packet, indent=2) + "\n", encoding="utf-8")
    print(f"H1-C7 regression packet passed: {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"H1-C7 regression capture failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
