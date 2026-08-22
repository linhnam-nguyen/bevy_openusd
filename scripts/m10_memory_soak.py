#!/usr/bin/env python3
"""Run the M10-C4 persistent Bevy runtime/cache soak in one process."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNTIME_ARTIFACT = ROOT / "target/benchmark/m10-c4-persistent-runtime.json"


def process_tree_rss_kb(root_pid: int) -> int:
    """Return RSS for the single cargo/test process tree owning the App."""
    try:
        listing = subprocess.check_output(
            ["ps", "-axo", "pid=,ppid=,rss="], text=True, stderr=subprocess.DEVNULL
        )
    except (OSError, subprocess.CalledProcessError):
        return 0

    children: dict[int, list[tuple[int, int]]] = {}
    rss_by_pid: dict[int, int] = {}
    for line in listing.splitlines():
        fields = line.split()
        if len(fields) != 3:
            continue
        try:
            pid, parent, rss = (int(field) for field in fields)
        except ValueError:
            continue
        rss_by_pid[pid] = rss
        children.setdefault(parent, []).append((pid, rss))

    total = rss_by_pid.get(root_pid, 0)
    pending = [root_pid]
    seen: set[int] = set()
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for child, rss in children.get(pid, []):
            total += rss
            pending.append(child)
    return total


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        default=str(ROOT / "target/benchmark/m10-c4-memory-soak.json"),
        help="JSON output path",
    )
    parser.add_argument("--cycles", type=int, default=12)
    args = parser.parse_args()
    if args.cycles < 12:
        raise SystemExit("M10-C4 requires at least twelve persistent cycles")

    command = [
        "cargo",
        "test",
        "--release",
        "--test",
        "m10_persistent_soak",
        "--",
        "--nocapture",
        "--test-threads=1",
    ]
    environment = os.environ.copy()
    environment["USDHUB_M10_C4_CYCLES"] = str(args.cycles)
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=environment,
        start_new_session=True,
    )
    high_water = 0
    samples = 0
    while process.poll() is None:
        high_water = max(high_water, process_tree_rss_kb(process.pid))
        samples += 1
        time.sleep(0.2)
    output = process.stdout.read() if process.stdout is not None else ""
    high_water = max(high_water, process_tree_rss_kb(process.pid))
    if process.returncode != 0:
        print(output[-8000:], file=sys.stderr)
        raise SystemExit(f"M10-C4 persistent runtime failed with exit code {process.returncode}")
    if not RUNTIME_ARTIFACT.exists():
        raise SystemExit(f"M10-C4 runtime artifact is missing: {RUNTIME_ARTIFACT}")

    runtime = json.loads(RUNTIME_ARTIFACT.read_text(encoding="utf-8"))
    if runtime.get("passed") is not True or runtime.get("persistent_app") is not True:
        raise SystemExit("M10-C4 runtime artifact did not prove a persistent App")
    result = {
        "schema": "usdhub.m10.c4.memory-soak.v2",
        "build_profile": "release",
        "runtime_mode": "one persistent Bevy App in one release test process",
        "rss_scope": "cargo/test process tree containing the persistent runtime",
        "command": command,
        "cycles": args.cycles,
        "duration_s": round(time.monotonic() - started, 3),
        "rss_high_water_kb": high_water,
        "rss_high_water_mb": round(high_water / 1024, 2),
        "rss_samples": samples,
        "runtime_artifact": str(RUNTIME_ARTIFACT),
        "runtime": runtime,
        "exit_code": process.returncode,
    }
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    runtime_copy = output_path.with_name("m10-c4-persistent-runtime.json")
    if runtime_copy.resolve() != RUNTIME_ARTIFACT.resolve():
        shutil.copyfile(RUNTIME_ARTIFACT, output_path.with_name("m10-c4-persistent-runtime.json"))
    print(f"M10-C4 persistent soak passed: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
