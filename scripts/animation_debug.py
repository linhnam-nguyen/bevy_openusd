#!/usr/bin/env python3
"""Run the B0-M0 animation diagnostics and emit one machine-readable report.

The command is intentionally diagnostic-first: it runs focused CPU/build gates
for the backend and frontend, records the exact branches and commits under
test, and never claims browser, GPU, Tauri, WebRTC, or production proof.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Sequence


SCHEMA_VERSION = 1
EXPECTED_BRANCH = "bug/animation-webview"
MAX_OUTPUT_CHARS = 2_000
ROOT = Path(__file__).resolve().parents[1]
FRONTEND_ROOT = ROOT.parent / "UsdHubUI"

FAILURE_CODES = {
    "branch": "BRANCH_MISMATCH",
    "missing_repository": "REPOSITORY_MISSING",
    "layout_missing": "SOURCE_LAYOUT_MISSING_FILE",
    "layout_failure": "SOURCE_LAYOUT_FAILURE",
    "backend_diff": "BACKEND_DIFF_CHECK_FAILED",
    "frontend_diff": "FRONTEND_DIFF_CHECK_FAILED",
    "backend_animation": "BACKEND_ANIMATION_TEST_FAILED",
    "backend_capture": "BACKEND_FRAME_CAPTURE_TEST_FAILED",
    "backend_signature": "BACKEND_FRAME_SIGNATURE_TEST_FAILED",
    "backend_streaming": "BACKEND_STREAMING_TEST_FAILED",
    "backend_compile": "BACKEND_COMPILE_FAILED",
    "frontend_webrtc": "FRONTEND_WEBRTC_TEST_FAILED",
    "frontend_wasm": "FRONTEND_WASM_CHECK_FAILED",
}


def tail(value: str) -> str:
    value = value.strip()
    return value[-MAX_OUTPUT_CHARS:] if value else ""


def command_result(
    check_id: str,
    command: Sequence[str],
    cwd: Path,
    failure_code: str,
    timeout_seconds: int = 600,
) -> dict[str, Any]:
    started = time.monotonic()
    try:
        completed = subprocess.run(
            list(command),
            cwd=cwd,
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_seconds,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return {
            "id": check_id,
            "status": "FAIL",
            "failure_code": failure_code,
            "command": list(command),
            "cwd": str(cwd),
            "exit_code": None,
            "duration_ms": round((time.monotonic() - started) * 1_000),
            "error": str(error),
        }

    result: dict[str, Any] = {
        "id": check_id,
        "status": "PASS" if completed.returncode == 0 else "FAIL",
        "failure_code": None if completed.returncode == 0 else failure_code,
        "command": list(command),
        "cwd": str(cwd),
        "exit_code": completed.returncode,
        "duration_ms": round((time.monotonic() - started) * 1_000),
    }
    if completed.returncode != 0:
        result["stdout_tail"] = tail(completed.stdout)
        result["stderr_tail"] = tail(completed.stderr)
    return result


def branch_result(repo_id: str, repo: Path) -> tuple[dict[str, Any], dict[str, str]]:
    if not repo.is_dir():
        return (
            {
                "id": f"{repo_id}_repository",
                "status": "FAIL",
                "failure_code": FAILURE_CODES["missing_repository"],
                "path": str(repo),
            },
            {},
        )

    branch = subprocess.run(
        ["git", "branch", "--show-current"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    sha = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    status = "PASS" if branch == EXPECTED_BRANCH else "FAIL"
    failure_code = None if status == "PASS" else FAILURE_CODES["branch"]
    result = {
        "id": f"{repo_id}_branch",
        "status": status,
        "failure_code": failure_code,
        "expected_branch": EXPECTED_BRANCH,
        "branch": branch,
        "sha": sha,
        "clean": not dirty,
        "path": str(repo),
    }
    if dirty:
        result["warning"] = "WORKTREE_DIRTY_AT_REPORT_TIME"
    return result, {"branch": branch, "sha": sha}


def layout_results() -> tuple[list[dict[str, Any]], list[str]]:
    files = {
        "backend_animation_tests": ROOT / "src/viewport/animation/tests.rs",
        "backend_frame_capture": ROOT / "src/viewport/transport/frame_capture.rs",
        "backend_frame_signature": ROOT / "src/viewport/transport/frame_signature.rs",
        "backend_metrics": ROOT / "crates/viewport_streaming/src/frame_metrics/mod.rs",
        "frontend_webrtc_stats": FRONTEND_ROOT / "apps/desktop/src/platform/webrtc/stats.rs",
    }
    results: list[dict[str, Any]] = []
    warnings: list[str] = []
    for check_id, path in files.items():
        if not path.is_file():
            results.append(
                {
                    "id": f"layout_{check_id}",
                    "status": "FAIL",
                    "failure_code": FAILURE_CODES["layout_missing"],
                    "path": str(path),
                }
            )
            continue
        with path.open("r", encoding="utf-8") as source:
            lines = sum(1 for _ in source)
        status = "FAIL" if lines > 400 else "PASS"
        failure_code = FAILURE_CODES["layout_failure"] if lines > 400 else None
        result = {
            "id": f"layout_{check_id}",
            "status": status,
            "failure_code": failure_code,
            "path": str(path),
            "lines": lines,
            "target": "200-350",
        }
        if 351 <= lines <= 400:
            result["status"] = "WARN"
            warnings.append(f"SOURCE_LAYOUT_WARNING:{path}:{lines}")
        results.append(result)
    return results, warnings


def decision_table() -> list[dict[str, Any]]:
    return [
        {
            "when": "all required checks are PASS; WARN entries are review-visible",
            "decision": "PASS",
            "failure_code": None,
        },
        {
            "when": "any required check is FAIL",
            "decision": "FAIL",
            "failure_code": "CHECK_FAILED",
        },
    ]


def build_report(output: Path) -> tuple[dict[str, Any], int]:
    checks: list[dict[str, Any]] = []
    evidence: dict[str, Any] = {}

    backend_branch, backend_identity = branch_result("backend", ROOT)
    frontend_branch, frontend_identity = branch_result("frontend", FRONTEND_ROOT)
    checks.extend([backend_branch, frontend_branch])
    evidence["backend"] = backend_identity
    evidence["frontend"] = frontend_identity

    layout, warnings = layout_results()
    checks.extend(layout)
    commands = [
        (
            "backend_diff",
            ("git", "diff", "--check"),
            ROOT,
            FAILURE_CODES["backend_diff"],
        ),
        (
            "frontend_diff",
            ("git", "diff", "--check"),
            FRONTEND_ROOT,
            FAILURE_CODES["frontend_diff"],
        ),
        (
            "backend_animation",
            ("cargo", "test", "-p", "usdview", "--lib", "viewport::animation"),
            ROOT,
            FAILURE_CODES["backend_animation"],
        ),
        (
            "backend_capture",
            (
                "cargo",
                "test",
                "-p",
                "usdview",
                "--lib",
                "viewport::transport::frame_capture",
            ),
            ROOT,
            FAILURE_CODES["backend_capture"],
        ),
        (
            "backend_signature",
            (
                "cargo",
                "test",
                "-p",
                "usdview",
                "--lib",
                "viewport::transport::frame_signature",
            ),
            ROOT,
            FAILURE_CODES["backend_signature"],
        ),
        (
            "backend_streaming",
            ("cargo", "test", "-p", "viewport_streaming"),
            ROOT,
            FAILURE_CODES["backend_streaming"],
        ),
        (
            "backend_compile",
            ("cargo", "check", "--bin", "usdview"),
            ROOT,
            FAILURE_CODES["backend_compile"],
        ),
        (
            "frontend_webrtc",
            ("cargo", "test", "-p", "usd_hub_desktop", "platform::webrtc::stats"),
            FRONTEND_ROOT,
            FAILURE_CODES["frontend_webrtc"],
        ),
        (
            "frontend_wasm",
            (
                "cargo",
                "check",
                "-p",
                "usd_hub_desktop",
                "--target",
                "wasm32-unknown-unknown",
            ),
            FRONTEND_ROOT,
            FAILURE_CODES["frontend_wasm"],
        ),
    ]
    for check_id, command, cwd, failure_code in commands:
        checks.append(command_result(check_id, command, cwd, failure_code))

    failures = sorted(
        {
            check["failure_code"]
            for check in checks
            if check.get("status") == "FAIL" and check.get("failure_code")
        }
    )
    status = "FAIL" if failures else "PASS"
    report = {
        "schema_version": SCHEMA_VERSION,
        "milestone": "B0-M0",
        "command": "make animation-debug",
        "status": status,
        "failure_codes": failures,
        "warnings": warnings,
        "evidence": evidence,
        "checks": checks,
        "decision_table": decision_table(),
        "runtime_proof_boundary": [
            "CPU tests and compile checks do not prove browser, GPU, Tauri, WebRTC, or production behavior."
        ],
        "report_path": str(output),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report, 1 if status == "FAIL" else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "target/animation-debug-report.json",
        help="JSON report path (default: target/animation-debug-report.json)",
    )
    args = parser.parse_args()
    output = args.output if args.output.is_absolute() else ROOT / args.output
    try:
        report, status = build_report(output)
    except (OSError, ValueError, TypeError) as error:
        print(f"animation-debug failed before report completion: {error}", file=sys.stderr)
        return 1
    print(
        f"animation-debug: {report['status']} "
        f"({len(report['failure_codes'])} failure codes, {len(report['warnings'])} warnings)"
    )
    print(f"report: {output}")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
