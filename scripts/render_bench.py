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
    "scenario_code",
    "connected",
    "server_hello_received",
    "session_ready",
    "video_received",
    "video_frames_observed",
    "command_sent",
    "authoritative_event_received",
    "client_state_reduced",
    "request_ids",
    "input_events_observed",
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


def validate_client_evidence(path: Path, scenario_id: str):
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
    ):
        if evidence[field] is not True:
            raise ValueError(f"client evidence field {field} is not true")
    if type(evidence["video_frames_observed"]) is not int or evidence["video_frames_observed"] <= 0:
        raise ValueError("client evidence must report at least one received video frame")
    if type(evidence["input_events_observed"]) is not int or evidence["input_events_observed"] < 0:
        raise ValueError("client evidence input_events_observed must be a non-negative integer")
    if not isinstance(evidence["request_ids"], list) or not all(
        isinstance(request_id, str) and request_id for request_id in evidence["request_ids"]
    ):
        raise ValueError("client evidence request_ids must be a list of non-empty strings")

    if scenario_id in EVENT_REQUIRED_SCENARIOS:
        if evidence["command_sent"] is not True or evidence["authoritative_event_received"] is not True:
            raise ValueError(
                f"{scenario_id} requires a client command and an authoritative event"
            )
        if not evidence["request_ids"]:
            raise ValueError(f"{scenario_id} requires at least one correlated request ID")
    elif scenario_id == "S15":
        if evidence["command_sent"] is not True or evidence["input_events_observed"] <= 0:
            raise ValueError("S15 requires client input traffic and observed input activity")
    elif evidence["command_sent"] is not False:
        raise ValueError(f"{scenario_id} must not claim an unconfigured client command")

def run_scenario(scenario_id: str, warmup: int, frames: int, output_path: str, label: str = "baseline", release: bool = True, force_headless: bool = False, fixture_override: str = None, client_command: str = None):
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

    cmd.extend([
        "--benchmark",
        "--benchmark-scenario", scenario_id,
        "--benchmark-warmup-frames", str(warmup),
        "--benchmark-frames", str(frames),
        "--benchmark-output", output_path,
        "--benchmark-label", label,
    ])

    fixture = fixture_override if fixture_override is not None else info.get("fixture")
    if fixture and os.path.exists(fixture):
        cmd.append(fixture)

    print(f"Running scenario {scenario_id} ({topology}): {info['desc']}...")
    client = None
    server = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        if info.get("client_required") and client_command:
            if not wait_for_signaling_server():
                print(f"WebRTC signaling server did not become ready for {scenario_id}", file=sys.stderr)
                return False
            client_evidence_path = Path(f"{output_path}.client.json").absolute()
            client_env = os.environ.copy()
            client_env.update({
                "USDHUB_BENCHMARK_SCENARIO": scenario_id,
                "USDHUB_BENCHMARK_OUTPUT": output_path,
                "USDHUB_BENCHMARK_LABEL": label,
                "USDHUB_BENCHMARK_EVIDENCE": str(client_evidence_path),
                "USDHUB_BENCHMARK_SIGNALING_URL": "ws://127.0.0.1:8080",
            })
            client = subprocess.Popen(shlex.split(client_command), env=client_env)
        server_stdout, server_stderr = server.communicate()
        if client is not None:
            try:
                client_returncode = client.wait(timeout=10)
            except subprocess.TimeoutExpired:
                client.terminate()
                client.wait(timeout=5)
                print(f"Client harness did not finish for {scenario_id}", file=sys.stderr)
                return False
            if client_returncode != 0:
                print(f"Client harness failed for {scenario_id} with exit code {client_returncode}", file=sys.stderr)
                return False
            try:
                validate_client_evidence(client_evidence_path, scenario_id)
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
    parser.add_argument(
        "--client-command",
        help="External UsdHubUI benchmark harness command for S12-S18; it must drive the live client and exit",
    )

    args = parser.parse_args()
    release = not args.debug

    if args.all:
        os.makedirs(args.output_dir, exist_ok=True)
        success = True
        for sc in sorted(SCENARIOS.keys(), key=lambda x: int(x[1:])):
            out_file = os.path.join(args.output_dir, f"{sc.lower()}.json")
            if not run_scenario(sc, args.warmup, args.frames, out_file, args.label, release, args.force_headless, args.fixture, args.client_command):
                success = False
        sys.exit(0 if success else 1)
    elif args.scenario:
        out_file = args.output or f"{args.scenario.lower()}.json"
        os.makedirs(os.path.dirname(os.path.abspath(out_file)), exist_ok=True)
        success = run_scenario(args.scenario, args.warmup, args.frames, out_file, args.label, release, args.force_headless, args.fixture, args.client_command)
        sys.exit(0 if success else 1)
    else:
        parser.print_help()
        sys.exit(1)

if __name__ == "__main__":
    main()
