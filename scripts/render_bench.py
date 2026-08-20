#!/usr/bin/env python3
"""
render_bench.py — Automated runner for Bevy OpenUSD rendering optimization benchmarks.

Usage:
    python3 scripts/render_bench.py --scenario S1 --output target/benchmark/s1.json
    python3 scripts/render_bench.py --all --output-dir target/benchmark/baseline/
"""

import argparse
import os
import shlex
import subprocess
import sys
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
            client_env = os.environ.copy()
            client_env.update({
                "USDHUB_BENCHMARK_SCENARIO": scenario_id,
                "USDHUB_BENCHMARK_OUTPUT": output_path,
                "USDHUB_BENCHMARK_LABEL": label,
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
    finally:
        if client is not None and client.poll() is None:
            client.terminate()
            client.wait(timeout=5)

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
