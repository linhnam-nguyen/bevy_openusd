#!/usr/bin/env python3
"""Run the M10 load/edit/resize soak and record process high-water memory."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RENDER_BENCH = ROOT / "scripts" / "render_bench.py"


def process_tree_rss_kb(root_pid: int) -> int:
    """Return the RSS sum for a process and all descendants, in KiB."""
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


def workload_commands() -> list[tuple[str, list[str]]]:
    python = sys.executable
    return [
        (
            "cache-load-kitchen",
            ["cargo", "test", "--release", "--test", "cache_profile", "--", "--nocapture"],
        ),
        (
            "point-instancer-edit",
            [
                "cargo",
                "test",
                "--release",
                "--test",
                "m8_instancing_correctness",
                "--test",
                "m8_instancing_profile",
                "--test",
                "m8_instancing_freeze",
                "--",
                "--nocapture",
            ],
        ),
        (
            "progressive-reload",
            [
                "cargo",
                "test",
                "--release",
                "--test",
                "progressive_load_profile",
                "--test",
                "subtree_resync_profile",
                "--",
                "--nocapture",
            ],
        ),
    ] + [
        (
            f"resize-{width}x{height}",
            [
                python,
                str(RENDER_BENCH),
                "--scenario",
                "S1",
                "--force-headless",
                "--warmup",
                "5",
                "--frames",
                "20",
                "--output",
                str(ROOT / "target" / "benchmark" / f"m10-c4-s1-{width}x{height}.json"),
                "--label",
                f"m10-c4-s1-{width}x{height}",
                "--stream-width",
                str(width),
                "--stream-height",
                str(height),
                "--stream-fps",
                "60",
            ],
        )
        for width, height in ((1280, 720), (1920, 1080), (2560, 1440))
    ]


def run_workload(label: str, command: list[str]) -> dict[str, object]:
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=os.environ.copy(),
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
    result = {
        "label": label,
        "command": command,
        "exit_code": process.returncode,
        "duration_s": round(time.monotonic() - started, 3),
        "rss_high_water_kb": high_water,
        "rss_high_water_mb": round(high_water / 1024, 2),
        "rss_samples": samples,
    }
    if process.returncode != 0:
        print(output[-4000:], file=sys.stderr)
        raise RuntimeError(f"M10-C4 workload failed: {label}")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        default=str(ROOT / "target" / "benchmark" / "m10-c4-memory-soak.json"),
        help="JSON output path",
    )
    args = parser.parse_args()

    reports = []
    for label, command in workload_commands():
        print(f"running {label}", flush=True)
        reports.append(run_workload(label, command))

    result = {
        "schema": "usdhub.m10.c4.memory-soak.v1",
        "build_profile": "release",
        "workloads": reports,
        "overall_rss_high_water_kb": max(
            (int(report["rss_high_water_kb"]) for report in reports), default=0
        ),
        "overall_rss_high_water_mb": round(
            max((int(report["rss_high_water_kb"]) for report in reports), default=0) / 1024,
            2,
        ),
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"M10-C4 artifact: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
