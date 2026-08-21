#!/usr/bin/env python3
"""
render_bench.py — Automated runner for Bevy OpenUSD rendering optimization benchmarks.

Usage:
    python3 scripts/render_bench.py --scenario S1 --output target/benchmark/s1.json
    python3 scripts/render_bench.py --all --output-dir target/benchmark/baseline/
"""

import argparse
import json
import os
import shlex
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path

SCENARIOS = {
    # Native steady-state & presentation regression probes (S1..S10)
    "S1": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Hummingbird Grid ON (paused)", "topology": "native"},
    "S2": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Hummingbird Grid OFF (paused)", "topology": "native"},
    "S3": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Camera Orbit/Pan", "topology": "native"},
    "S4": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Grid Visibility Toggle", "topology": "native"},
    "S5": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Ground Origin Change", "topology": "native"},
    "S6": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Grid Style Color Change", "topology": "native"},
    "S7": {"fixture": "tests/stages/empty.usda", "desc": "Native Visually Empty LiveStage Retained", "topology": "native"},
    "S8": {"fixture": None, "desc": "Native No LiveStage", "topology": "native"},
    "S9": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Recovery Idle", "topology": "native"},
    "S10": {"fixture": "assets/external/hummingbird.usdz", "desc": "Native Authoritative USD Change", "topology": "native"},

    # WebRTC remote path (S11..S18)
    "S11": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Headless Server Idle (no client)", "topology": "webrtc"},
    "S12": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Client Connected Idle", "topology": "webrtc", "client_required": True},
    "S13": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Grid Visibility Command", "topology": "webrtc", "client_required": True},
    "S14": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Ground Origin Command", "topology": "webrtc", "client_required": True},
    "S15": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Orbit/Pan Input", "topology": "webrtc", "client_required": True},
    "S16": {"fixture": "tests/stages/empty.usda", "desc": "WebRTC Remote Visually Empty Stage", "topology": "webrtc", "client_required": True},
    "S17": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Authoritative USD Edit", "topology": "webrtc", "client_required": True},
    "S18": {"fixture": "assets/external/hummingbird.usdz", "desc": "WebRTC Remote Command After Long Idle", "topology": "webrtc", "client_required": True},

    # Render / Data Plane Isolation (S19..S24)
    "S19": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Query Saturation", "topology": "isolation"},
    "S20": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Auth Validation Burst", "topology": "isolation"},
    "S21": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Navigation Under Auth", "topology": "isolation"},
    "S22": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Query Command Concurrency", "topology": "isolation"},
    "S23": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Slow/Failing Data Worker", "topology": "isolation"},
    "S24": {"fixture": "assets/external/hummingbird.usdz", "desc": "Isolation Auth Revocation Propagation", "topology": "isolation"},
}

CLIENT_EVIDENCE_FIELDS = [
    "run_id",
    "scenario_code",
    "connected",
    "server_hello_received",
    "session_ready",
    "video_received",
    "video_frames_observed",
    "video_frames_during_measurement",
    "measurement_started",
    "measurement_complete",
    "measurement_idle_observed",
    "command_sent",
    "command_enqueue_observed",
    "authoritative_event_received",
    "client_event_receipt_observed",
    "client_state_reduced",
    "request_ids",
    "matched_request_ids",
    "input_events_observed",
    "orbit_pan_events_observed",
    "zoom_dolly_events_observed",
    "zoom_delta_observed",
    "video_observation",
    "stream_configuration",
    "completion_blockers",
]

EVENT_REQUIRED_SCENARIOS = {"S13", "S14", "S17", "S18"}


def wait_for_signaling_server(host: str = "127.0.0.1", port: int = 8080, timeout: float = 30.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.25):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def validate_client_evidence(path: Path, scenario_id: str, run_id: str):
    if not path.exists():
        raise ValueError(
            f"client harness did not write the required evidence artifact: {path}"
        )
    try:
        evidence = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid client evidence artifact {path}: {error}") from error

    missing = [field for field in CLIENT_EVIDENCE_FIELDS if field not in evidence]
    if missing:
        raise ValueError(f"client evidence is missing fields: {', '.join(missing)}")
    if evidence["run_id"] != run_id:
        raise ValueError("client evidence run ID does not match the current benchmark run")
    if evidence["scenario_code"] != scenario_id:
        raise ValueError(
            f"client evidence scenario mismatch: {evidence['scenario_code']} != {scenario_id}"
        )
    for field in (
        "connected",
        "server_hello_received",
        "session_ready",
        "video_received",
        "client_state_reduced",
        "measurement_started",
        "measurement_complete",
    ):
        if evidence[field] is not True:
            raise ValueError(f"client evidence field {field} is not true")
    if type(evidence["video_frames_observed"]) is not int or evidence["video_frames_observed"] <= 0:
        raise ValueError("client evidence must report at least one received video frame")
    if type(evidence["video_frames_during_measurement"]) is not int or evidence["video_frames_during_measurement"] <= 0:
        raise ValueError("client evidence must report video frames during the measured window")
    video_observation = evidence["video_observation"]
    if not isinstance(video_observation, dict):
        raise ValueError("client evidence video_observation must be an object")
    for field in ("decoded_width", "decoded_height"):
        if type(video_observation.get(field)) is not int or video_observation[field] <= 0:
            raise ValueError(f"client evidence must report positive decoded video {field[8:]}")
    for field in (
        "decoded_fps",
        "decode_ms",
        "estimated_network_plus_decode_ms",
        "network_rtt_ms",
        "network_jitter_ms",
    ):
        value = video_observation.get(field)
        if value is not None and (type(value) not in (int, float) or value < 0):
            raise ValueError(f"client evidence {field} must be a non-negative number or null")
    if type(video_observation.get("dropped_frames")) is not int or video_observation["dropped_frames"] < 0:
        raise ValueError("client evidence dropped_frames must be a non-negative integer")
    if evidence["completion_blockers"] != []:
        raise ValueError(
            "client evidence reports incomplete benchmark state: "
            + ", ".join(evidence["completion_blockers"])
        )
    stream_configuration = evidence["stream_configuration"]
    if not isinstance(stream_configuration, dict):
        raise ValueError("client evidence stream_configuration must be an object")
    required_configuration_fields = (
        "requested_width",
        "requested_height",
        "requested_fps",
        "accepted_width",
        "accepted_height",
        "accepted_fps",
        "accepted_generation",
        "applied_width",
        "applied_height",
        "applied_fps",
        "applied_generation",
    )
    if any(stream_configuration.get(field) is None for field in required_configuration_fields):
        raise ValueError("client evidence must report the complete stream configuration chain")
    if (
        stream_configuration["requested_width"] != stream_configuration["accepted_width"]
        or stream_configuration["requested_height"] != stream_configuration["accepted_height"]
        or stream_configuration["requested_fps"] != stream_configuration["accepted_fps"]
        or stream_configuration["accepted_width"] != stream_configuration["applied_width"]
        or stream_configuration["accepted_height"] != stream_configuration["applied_height"]
        or stream_configuration["accepted_fps"] != stream_configuration["applied_fps"]
        or stream_configuration["accepted_generation"] != stream_configuration["applied_generation"]
        or video_observation["decoded_width"] != stream_configuration["applied_width"]
        or video_observation["decoded_height"] != stream_configuration["applied_height"]
    ):
        raise ValueError("client evidence stream configuration does not match decoded video")
    if type(evidence["measurement_idle_observed"]) is not bool:
        raise ValueError("client evidence measurement_idle_observed must be boolean")
    if type(evidence["input_events_observed"]) is not int or evidence["input_events_observed"] < 0:
        raise ValueError("client evidence input_events_observed must be a non-negative integer")
    if not isinstance(evidence["request_ids"], list) or not all(
        isinstance(request_id, str) and request_id for request_id in evidence["request_ids"]
    ):
        raise ValueError("client evidence request_ids must be a list of non-empty strings")

    if scenario_id in EVENT_REQUIRED_SCENARIOS:
        if evidence["command_sent"] is not True or evidence["command_enqueue_observed"] is not True or evidence["authoritative_event_received"] is not True or evidence["client_event_receipt_observed"] is not True:
            raise ValueError(
                f"{scenario_id} requires a client command and an authoritative event"
            )
        if not evidence["request_ids"]:
            raise ValueError(f"{scenario_id} requires at least one correlated request ID")
        if scenario_id in {"S13", "S14"} and len(evidence["matched_request_ids"]) < 2:
            raise ValueError(f"{scenario_id} requires both ordered client commands")
        if scenario_id == "S18" and evidence["measurement_idle_observed"] is not True:
            raise ValueError("S18 requires the measured idle barrier before its command")
    elif scenario_id == "S15":
        if evidence["command_sent"] is not True or evidence["command_enqueue_observed"] is not True or evidence["input_events_observed"] <= 0:
            raise ValueError("S15 requires client input traffic and observed input activity")
        if evidence["orbit_pan_events_observed"] <= 0 or evidence["zoom_dolly_events_observed"] <= 0 or evidence["zoom_delta_observed"] == 0:
            raise ValueError("S15 requires separate orbit/pan and zoom/dolly input phases")
    elif evidence["command_sent"] is not False:
        raise ValueError(f"{scenario_id} must not claim an unconfigured client command")

def run_scenario(scenario_id: str, warmup: int, frames: int, output_path: str, label: str = "baseline", release: bool = True, force_headless: bool = False, fixture_override: str = None, client_command: str = None, stream_width: int = None, stream_height: int = None, stream_fps: int = None):
    info = SCENARIOS.get(scenario_id)
    if not info:
        print(f"Error: Unknown scenario {scenario_id}", file=sys.stderr)
        return False

    cmd = ["cargo", "run"]
    if release:
        cmd.append("--release")
    cmd.extend(["--bin", "usdview", "--"])

    topology = info.get("topology", "native")
    if info.get("client_required") and not client_command:
        print(
            f"Error: {scenario_id} requires --client-command for a real UsdHubUI round-trip; "
            "server-only output is not accepted as client evidence.",
            file=sys.stderr,
        )
        return False
    if topology in ("webrtc", "isolation"):
        cmd.extend(["--headless", "--webrtc"])
    else:
        # Native scenarios run real native Frost composition unless force_headless requested
        if force_headless:
            cmd.append("--headless")

    client_required = bool(info.get("client_required"))
    run_id = None
    client_evidence_path = None
    client_diagnostic_path = None
    ready_file = None
    measurement_start_file = None
    measurement_idle_file = None
    measurement_complete_file = None
    if client_required:
        run_id = uuid.uuid4().hex
        client_evidence_path = Path(f"{output_path}.client.json").absolute()
        client_diagnostic_path = Path(f"{output_path}.client.json.diagnostic.json").absolute()
        ready_file = Path(f"{output_path}.client-ready").absolute()
        measurement_start_file = Path(f"{output_path}.measurement-start").absolute()
        measurement_idle_file = Path(f"{output_path}.measurement-idle").absolute()
        measurement_complete_file = Path(f"{output_path}.measurement-complete").absolute()
        for marker in (
            ready_file,
            measurement_start_file,
            measurement_idle_file,
            measurement_complete_file,
            client_diagnostic_path,
        ):
            marker.unlink(missing_ok=True)

    cmd.extend([
        "--benchmark",
        "--benchmark-scenario", scenario_id,
        "--benchmark-warmup-frames", str(warmup),
        "--benchmark-frames", str(frames),
        "--benchmark-output", output_path,
        "--benchmark-label", label,
    ])
    stream_width = stream_width or 1920
    stream_height = stream_height or 1080
    stream_fps = stream_fps or 60
    cmd.extend(["--width", str(stream_width), "--height", str(stream_height), "--fps", str(stream_fps)])
    if client_required:
        cmd.extend([
            "--benchmark-client-ready-file", str(ready_file),
            "--benchmark-measurement-start-file", str(measurement_start_file),
            "--benchmark-measurement-idle-file", str(measurement_idle_file),
            "--benchmark-measurement-complete-file", str(measurement_complete_file),
        ])

    fixture = fixture_override if fixture_override is not None else info.get("fixture")
    if fixture and os.path.exists(fixture):
        cmd.append(fixture)

    print(f"Running scenario {scenario_id} ({topology}): {info['desc']}...")
    client = None
    server = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        if client_required and client_command:
            if not wait_for_signaling_server():
                print(f"WebRTC signaling server did not become ready for {scenario_id}", file=sys.stderr)
                return False
            client_env = os.environ.copy()
            client_env.update({
                "USDHUB_BENCHMARK_RUN_ID": run_id,
                "USDHUB_BENCHMARK_SCENARIO": scenario_id,
                "USDHUB_BENCHMARK_OUTPUT": output_path,
                "USDHUB_BENCHMARK_LABEL": label,
                "USDHUB_BENCHMARK_EVIDENCE": str(client_evidence_path),
                "USDHUB_BENCHMARK_SIGNALING_URL": "ws://127.0.0.1:8080",
                "USDHUB_BENCHMARK_READY_FILE": str(ready_file),
                "USDHUB_BENCHMARK_MEASUREMENT_START_FILE": str(measurement_start_file),
                "USDHUB_BENCHMARK_MEASUREMENT_IDLE_FILE": str(measurement_idle_file),
                "USDHUB_BENCHMARK_MEASUREMENT_COMPLETE_FILE": str(measurement_complete_file),
                "USDHUB_BENCHMARK_REQUESTED_WIDTH": str(stream_width),
                "USDHUB_BENCHMARK_REQUESTED_HEIGHT": str(stream_height),
                "USDHUB_BENCHMARK_REQUESTED_FPS": str(stream_fps),
            })
            client = subprocess.Popen(shlex.split(client_command), env=client_env)
        server_stdout, server_stderr = server.communicate()
        if client is not None:
            try:
                client_returncode = client.wait(
                    timeout=float(os.environ.get("USDHUB_BENCHMARK_CLIENT_TIMEOUT", "60"))
                )
            except subprocess.TimeoutExpired:
                client.terminate()
                client.wait(timeout=5)
                if client_diagnostic_path.exists():
                    print(
                        f"Client diagnostic for {scenario_id}: "
                        f"{client_diagnostic_path.read_text(encoding='utf-8')}",
                        file=sys.stderr,
                    )
                print(f"Client harness did not finish for {scenario_id}", file=sys.stderr)
                return False
            if client_returncode != 0:
                print(f"Client harness failed for {scenario_id} with exit code {client_returncode}", file=sys.stderr)
                return False
            try:
                validate_client_evidence(client_evidence_path, scenario_id, run_id)
            except ValueError as error:
                print(str(error), file=sys.stderr)
                return False
    finally:
        if client is not None and client.poll() is None:
            client.terminate()
            client.wait(timeout=5)
        if server.poll() is None:
            server.terminate()
            server.wait(timeout=5)

    if server.returncode != 0:
        print(f"Execution failed for {scenario_id}:\n{server_stderr}", file=sys.stderr)
        return False

    print(f"✓ {scenario_id} report written to {output_path}")
    return True

def main():
    parser = argparse.ArgumentParser(description="Render optimization benchmark runner")
    parser.add_argument("--scenario", help="Scenario ID to run (e.g. S1)")
    parser.add_argument("--fixture", help="Optional fixture override (e.g. assets/external/Kitchen_set.usdz)")
    parser.add_argument("--all", action="store_true", help="Run all S1..S24 scenarios")
    parser.add_argument("--warmup", type=int, default=30, help="Warmup frame count")
    parser.add_argument("--frames", type=int, default=120, help="Measured frame count")
    parser.add_argument("--output", help="Single report output path")
    parser.add_argument("--output-dir", default="target/benchmark/baseline", help="Directory for multi-scenario output")
    parser.add_argument("--label", default="baseline", help="Benchmark run label")
    parser.add_argument("--debug", action="store_true", help="Run debug build instead of release")
    parser.add_argument("--force-headless", action="store_true", help="Force headless execution for native scenarios")
    parser.add_argument("--stream-width", type=int, help="Requested WebRTC stream width for S12-S18")
    parser.add_argument("--stream-height", type=int, help="Requested WebRTC stream height for S12-S18")
    parser.add_argument("--stream-fps", type=int, help="Requested WebRTC stream FPS for S12-S18")
    parser.add_argument(
        "--configuration-matrix",
        action="store_true",
        help="Run the focused real-client S12 stream configuration matrix",
    )
    parser.add_argument(
        "--client-command",
        help="External UsdHubUI benchmark harness command for S12-S18; it must drive the live client and exit",
    )

    args = parser.parse_args()
    release = not args.debug

    if args.configuration_matrix:
        if not args.client_command:
            parser.error("--configuration-matrix requires --client-command")
        os.makedirs(args.output_dir, exist_ok=True)
        cases = [
            (1280, 720, 30), (1280, 720, 60), (1280, 720, 120),
            (1920, 1080, 30), (1920, 1080, 60), (1920, 1080, 120),
            (2560, 1440, 30), (2560, 1440, 60), (2560, 1440, 120),
        ]
        results = []
        for width, height, fps in cases:
            stem = f"s12-{width}x{height}-{fps}fps"
            output_path = os.path.join(args.output_dir, f"{stem}.json")
            passed = run_scenario(
                "S12", args.warmup, args.frames, output_path, args.label, release,
                args.force_headless, args.fixture, args.client_command, width, height, fps,
            )
            results.append({
                "width": width,
                "height": height,
                "requested_fps": fps,
                "status": "passed" if passed else "failed",
                "server_report": output_path,
                "client_report": f"{output_path}.client.json",
            })
        summary = {
            "schema_version": 1,
            "scenario": "S12",
            "cases": results,
            "unsupported_cases": [],
            "support_status": "all_requested_cases_executed",
        }
        Path(args.output_dir, "configuration-matrix.json").write_text(
            json.dumps(summary, indent=2) + "\n", encoding="utf-8"
        )
        sys.exit(0 if all(result["status"] == "passed" for result in results) else 1)

    if args.all:
        os.makedirs(args.output_dir, exist_ok=True)
        success = True
        for sc in sorted(SCENARIOS.keys(), key=lambda x: int(x[1:])):
            out_file = os.path.join(args.output_dir, f"{sc.lower()}.json")
            if not run_scenario(sc, args.warmup, args.frames, out_file, args.label, release, args.force_headless, args.fixture, args.client_command, args.stream_width, args.stream_height, args.stream_fps):
                success = False
        sys.exit(0 if success else 1)
    elif args.scenario:
        out_file = args.output or f"{args.scenario.lower()}.json"
        os.makedirs(os.path.dirname(os.path.abspath(out_file)), exist_ok=True)
        success = run_scenario(args.scenario, args.warmup, args.frames, out_file, args.label, release, args.force_headless, args.fixture, args.client_command, args.stream_width, args.stream_height, args.stream_fps)
        sys.exit(0 if success else 1)
    else:
        parser.print_help()
        sys.exit(1)

if __name__ == "__main__":
    main()
