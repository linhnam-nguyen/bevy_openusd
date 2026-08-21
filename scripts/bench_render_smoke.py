#!/usr/bin/env python3
"""Run one fresh headless release render smoke and validate its invariants."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "target" / "benchmark" / "m10-c6-render-smoke.json"


def main() -> int:
    command = [
        sys.executable,
        str(ROOT / "scripts" / "render_bench.py"),
        "--scenario",
        "S1",
        "--force-headless",
        "--warmup",
        "5",
        "--frames",
        "20",
        "--output",
        str(OUTPUT),
        "--label",
        "m10-c6-render-smoke",
    ]
    subprocess.run(command, cwd=ROOT, check=True)
    report = json.loads(OUTPUT.read_text(encoding="utf-8"))
    grid = report["incident_grid"]
    semantic = report["incident_semantic"]
    if report["configuration_matches"] is not True or report["steady_state_matches"] is not True:
        raise SystemExit("render smoke did not reach the expected effective state")
    if grid["structural_rebuilds"] != 0 or semantic["snapshot_clones"] != 0:
        raise SystemExit("render smoke violated idle structural or semantic invariants")
    print(f"M10-C6 render smoke passed: {OUTPUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
