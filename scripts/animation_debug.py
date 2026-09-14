#!/usr/bin/env python3
"""Run the B0-M0 animation evidence matrix without inventing proof.

Each required matrix row is either backed by its named scenario, explicitly
UNAVAILABLE, or ERROR when the scenario is missing. The command deliberately
returns a red report while browser/WebRTC and fault-injection harnesses are
not available; CPU tests and a backend process cannot stand in for them.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Sequence


SCHEMA_VERSION = 2
EXPECTED_BRANCH = "bug/animation-webview"
MAX_OUTPUT_CHARS = 2_000
ROOT = Path(__file__).resolve().parents[1]
FRONTEND_ROOT = ROOT.parent / "UsdHubUI"

PASS = "PASS"
FAIL = "FAIL"
WARN = "WARN"
UNAVAILABLE = "UNAVAILABLE"
ERROR = "ERROR"
VALID_STATUSES = {PASS, FAIL, WARN, UNAVAILABLE, ERROR}

MATRIX = (
    {
        "id": "A",
        "description": "real Hummingbird native/backend stage evaluation",
        "scenario": "scenario_a_hummingbird_stage",
    },
    {
        "id": "B",
        "description": "real Hummingbird headless/offscreen render signature",
        "scenario": "scenario_b_hummingbird_render",
    },
    {
        "id": "C",
        "description": "real Hummingbird WebRTC/Tauri delivery and presentation",
        "scenario": "scenario_c_hummingbird_client",
    },
    {
        "id": "D",
        "description": "static stage negative animation control",
        "scenario": "scenario_d_static",
    },
    {
        "id": "E",
        "description": "Hummingbird to static to Hummingbird replacement",
        "scenario": "scenario_e_replacement",
    },
    {
        "id": "F",
        "description": "cache-first activation to canonical LiveStage Ready",
        "scenario": "scenario_f_cache_first",
    },
    {
        "id": "G",
        "description": "paused deterministic t0 to t1 to t0 round trip",
        "scenario": "scenario_g_seek_round_trip",
    },
)

SUPPLEMENTAL_REQUIRED = (
    "protocol_debug_message_size",
    "existing_animation_tests",
    "existing_project_activation_tests",
    "existing_transport_tests",
    "frontend_webrtc_tests",
    "frontend_wasm_check",
    "animation_debug_runtime",
)


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
            "status": FAIL,
            "failure_code": failure_code,
            "command": list(command),
            "cwd": str(cwd),
            "exit_code": None,
            "duration_ms": round((time.monotonic() - started) * 1_000),
            "error": str(error),
        }

    result: dict[str, Any] = {
        "id": check_id,
        "status": PASS if completed.returncode == 0 else FAIL,
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


def unavailable(check_id: str, reason: str, failure_code: str) -> dict[str, Any]:
    return {
        "id": check_id,
        "status": UNAVAILABLE,
        "failure_code": failure_code,
        "reason": reason,
    }


def matrix_row(row_id: str, description: str, scenario: dict[str, Any] | None) -> dict[str, Any]:
    """Convert one named scenario into a strict matrix row."""
    if scenario is None:
        return {
            "id": row_id,
            "description": description,
            "status": ERROR,
            "failure_code": "MATRIX_SCENARIO_MISSING",
        }
    status = scenario.get("status")
    if status not in VALID_STATUSES:
        return {
            "id": row_id,
            "description": description,
            "status": ERROR,
            "failure_code": "MATRIX_SCENARIO_INVALID_STATUS",
            "scenario": scenario,
        }
    return {
        "id": row_id,
        "description": description,
        "scenario": scenario.get("id"),
        "status": status,
        "failure_code": scenario.get("failure_code"),
        "evidence": scenario.get("evidence"),
    }


def overall_status(
    matrix: Sequence[dict[str, Any]],
    supplemental: Sequence[dict[str, Any]],
    preflight: Sequence[dict[str, Any]],
    fault_report: dict[str, Any],
) -> str:
    matrix_pass = all(row.get("status") == PASS for row in matrix)
    supplemental_pass = all(check.get("status") == PASS for check in supplemental)
    preflight_pass = all(check.get("status") in {PASS, WARN} for check in preflight)
    fault_pass = fault_report.get("status") == PASS
    return PASS if matrix_pass and supplemental_pass and preflight_pass and fault_pass else FAIL


def branch_result(repo_id: str, repo: Path) -> dict[str, Any]:
    if not repo.is_dir():
        return {
            "id": f"{repo_id}_repository",
            "status": FAIL,
            "failure_code": "REPOSITORY_MISSING",
            "path": str(repo),
        }
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
    return {
        "id": f"{repo_id}_branch",
        "status": PASS if branch == EXPECTED_BRANCH else FAIL,
        "failure_code": None if branch == EXPECTED_BRANCH else "BRANCH_MISMATCH",
        "expected_branch": EXPECTED_BRANCH,
        "branch": branch,
        "sha": sha,
        "clean": not dirty,
        "path": str(repo),
    }


def layout_results() -> list[dict[str, Any]]:
    files = {
        "backend_animation_tests": ROOT / "src/viewport/animation/tests.rs",
        "backend_frame_capture": ROOT / "src/viewport/transport/frame_capture.rs",
        "backend_frame_signature": ROOT / "src/viewport/transport/frame_signature.rs",
        "backend_metrics": ROOT / "crates/viewport_streaming/src/frame_metrics/mod.rs",
        "frontend_webrtc_stats": FRONTEND_ROOT / "apps/desktop/src/platform/webrtc/stats.rs",
    }
    results: list[dict[str, Any]] = []
    for check_id, path in files.items():
        if not path.is_file():
            results.append(
                {
                    "id": f"layout_{check_id}",
                    "status": FAIL,
                    "failure_code": "SOURCE_LAYOUT_MISSING_FILE",
                    "path": str(path),
                }
            )
            continue
        lines = sum(1 for _ in path.open("r", encoding="utf-8"))
        status = FAIL if lines > 400 else WARN if lines >= 351 else PASS
        results.append(
            {
                "id": f"layout_{check_id}",
                "status": status,
                "failure_code": "SOURCE_LAYOUT_FAILURE" if lines > 400 else None,
                "path": str(path),
                "lines": lines,
                "target": "200-350",
            }
        )
    return results


def run_preflight() -> list[dict[str, Any]]:
    checks = [branch_result("backend", ROOT), branch_result("frontend", FRONTEND_ROOT)]
    checks.extend(layout_results())
    checks.extend(
        [
            command_result("backend_diff", ["git", "diff", "--check"], ROOT, "BACKEND_DIFF_CHECK_FAILED"),
            command_result("frontend_diff", ["git", "diff", "--check"], FRONTEND_ROOT, "FRONTEND_DIFF_CHECK_FAILED"),
            command_result(
                "backend_compile",
                ["cargo", "check", "--bin", "usdview"],
                ROOT,
                "BACKEND_COMPILE_FAILED",
                timeout_seconds=900,
            ),
            command_result(
                "frontend_wasm_check",
                ["cargo", "check", "-p", "usd_hub_desktop", "--target", "wasm32-unknown-unknown"],
                FRONTEND_ROOT,
                "FRONTEND_WASM_CHECK_FAILED",
                timeout_seconds=900,
            ),
        ]
    )
    return checks


def focused_test(check_id: str, filter_name: str, failure_code: str) -> dict[str, Any]:
    return command_result(
        check_id,
        ["cargo", "test", "-p", "usdview", "--lib", filter_name],
        ROOT,
        failure_code,
        timeout_seconds=900,
    )


def run_backend_render(output: Path) -> tuple[dict[str, Any], dict[str, Any] | None]:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.unlink(missing_ok=True)
    command = [
        "cargo",
        "run",
        "--bin",
        "usdview",
        "--",
        "--headless",
        "--webrtc",
        "--width",
        "640",
        "--height",
        "480",
        "--fps",
        "30",
        "--animation-debug",
        "--animation-debug-output",
        str(output),
        "assets/external/hummingbird.usdz",
    ]
    command_check = command_result(
        "scenario_b_hummingbird_render_command",
        command,
        ROOT,
        "HEADLESS_RENDER_COMMAND_FAILED",
        timeout_seconds=240,
    )
    if not output.is_file():
        command_check["status"] = FAIL
        command_check["failure_code"] = "HEADLESS_RENDER_REPORT_MISSING"
        command_check["error"] = "backend diagnostic did not write its JSON report"
        return command_check, None
    try:
        report = json.loads(output.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        command_check["status"] = FAIL
        command_check["failure_code"] = "HEADLESS_RENDER_REPORT_INVALID"
        command_check["error"] = str(error)
        return command_check, None
    command_check["report_schema_version"] = report.get("schema_version")
    command_check["backend_report_status"] = report.get("pass")
    return command_check, report


def hummingbird_render_scenario(report: dict[str, Any] | None, command_check: dict[str, Any]) -> dict[str, Any]:
    if report is None:
        return {
            "id": "scenario_b_hummingbird_render",
            "status": FAIL,
            "failure_code": command_check.get("failure_code"),
        }
    server = report.get("server_evidence", {})
    pairwise = report.get("pairwise_render_mad_luma", {})
    t0 = server.get("t0") or {}
    t1 = server.get("t1") or {}
    required = {
        "stage_ready": t0.get("stage_ready") is True and t1.get("stage_ready") is True,
        "animated_prim_count": (report.get("animated_prim_count") or 0) > 0,
        "hash_t0_differs_from_t1": server.get("hash_t0_differs_from_t1") is True,
        "sequence_t1_after_t0": server.get("sequence_t1_after_t0") is True,
        "sequence_round_trip_after_t1": server.get("sequence_round_trip_after_t1") is True,
        "hummingbird_t0_t1_mad": isinstance(pairwise.get("hummingbird_t0_t1"), (int, float)),
        "hummingbird_round_trip_mad": isinstance(pairwise.get("hummingbird_t0_round_trip"), (int, float)),
    }
    status = PASS if all(required.values()) and command_check.get("exit_code") == 0 else FAIL
    return {
        "id": "scenario_b_hummingbird_render",
        "status": status,
        "failure_code": None if status == PASS else "HEADLESS_RENDER_EVIDENCE_INCOMPLETE",
        "evidence": {"required": required, "backend_report": report},
    }


def fault_verification(requested: str | None, expected: str | None = None, observed: str | None = None) -> dict[str, Any]:
    if requested is None:
        return unavailable(
            "fault_injection",
            "No real fault-injection harness is registered for B0-M0; normal evidence was not mutated.",
            "FAULT_INJECTION_UNAVAILABLE",
        )
    if expected is None or observed is None:
        return unavailable(
            "fault_injection",
            f"Requested fault {requested!r}, but no executable harness is available.",
            "FAULT_INJECTION_UNAVAILABLE",
        )
    status = PASS if expected == observed else FAIL
    return {
        "id": "fault_injection",
        "status": status,
        "failure_code": None if status == PASS else "FAULT_CLASSIFICATION_MISMATCH",
        "requested": requested,
        "expected": expected,
        "observed": observed,
    }


def decision_table() -> list[dict[str, Any]]:
    return [
        {"when": "all A-G rows are PASS", "decision": "eligible", "failure_code": None},
        {"when": "any required row is UNAVAILABLE or ERROR", "decision": "FAIL", "failure_code": "MATRIX_EVIDENCE_MISSING"},
        {"when": "fault verification is UNAVAILABLE", "decision": "FAIL", "failure_code": "FAULT_INJECTION_UNAVAILABLE"},
        {"when": "preflight or supplemental check is FAIL", "decision": "FAIL", "failure_code": "CHECK_FAILED"},
    ]


def build_report(output: Path, requested_fault: str | None = None) -> tuple[dict[str, Any], int]:
    preflight = run_preflight()
    scenarios: dict[str, dict[str, Any]] = {}
    scenarios["scenario_a_hummingbird_stage"] = focused_test(
        "scenario_a_hummingbird_stage",
        "viewport::animation::tests::hummingbird_native_stage_evaluation_is_animated",
        "NATIVE_STAGE_EVALUATION_FAILED",
    )
    scenarios["scenario_d_static"] = focused_test(
        "scenario_d_static",
        "viewport::animation::tests::static_stage_is_negative_animation_control",
        "STATIC_NEGATIVE_CONTROL_FAILED",
    )
    scenarios["scenario_e_replacement"] = focused_test(
        "scenario_e_replacement",
        "viewport::animation::tests::hummingbird_static_hummingbird_replacement_restores_animation",
        "STAGE_REPLACEMENT_FAILED",
    )
    scenarios["scenario_f_cache_first"] = focused_test(
        "scenario_f_cache_first",
        "viewport::app::project_activation::production_tests::cache_first_tests::cache_first_activation_does_not_fake_canonical_animation_readiness",
        "CACHE_FIRST_CANONICAL_READINESS_FAILED",
    )
    scenarios["scenario_g_seek_round_trip"] = focused_test(
        "scenario_g_seek_round_trip",
        "viewport::animation::tests::hummingbird_paused_seek_round_trip_has_stable_transform_signature",
        "PAUSED_SEEK_ROUND_TRIP_FAILED",
    )

    backend_report_path = ROOT / "target/animation-debug-backend.json"
    render_command, backend_report = run_backend_render(backend_report_path)
    scenarios["scenario_b_hummingbird_render"] = hummingbird_render_scenario(backend_report, render_command)
    scenarios["scenario_c_hummingbird_client"] = unavailable(
        "scenario_c_hummingbird_client",
        "No automated real Tauri/browser WebRTC client scenario is available in this workspace.",
        "CLIENT_E2E_UNAVAILABLE",
    )

    matrix = [
        matrix_row(entry["id"], entry["description"], scenarios.get(entry["scenario"]))
        for entry in MATRIX
    ]
    supplemental = [
        command_result(
            "protocol_debug_message_size",
            ["cargo", "test", "-p", "viewport_protocol", "animation_debug::tests::actual_debug_event_stays_below_one_kibibyte"],
            ROOT,
            "PROTOCOL_DEBUG_MESSAGE_SIZE_FAILED",
        ),
        command_result(
            "existing_animation_tests",
            ["cargo", "test", "-p", "usdview", "--lib", "viewport::animation"],
            ROOT,
            "EXISTING_ANIMATION_TESTS_FAILED",
            timeout_seconds=900,
        ),
        command_result(
            "existing_project_activation_tests",
            ["cargo", "test", "-p", "usdview", "--lib", "viewport::app::project_activation"],
            ROOT,
            "EXISTING_PROJECT_ACTIVATION_TESTS_FAILED",
            timeout_seconds=900,
        ),
        command_result(
            "existing_transport_tests",
            ["cargo", "test", "-p", "usdview", "--lib", "viewport::transport"],
            ROOT,
            "EXISTING_TRANSPORT_TESTS_FAILED",
            timeout_seconds=900,
        ),
        command_result(
            "frontend_webrtc_tests",
            ["cargo", "test", "-p", "usd_hub_desktop", "platform::webrtc::stats"],
            FRONTEND_ROOT,
            "FRONTEND_WEBRTC_TESTS_FAILED",
            timeout_seconds=900,
        ),
        next(
            (check for check in preflight if check["id"] == "frontend_wasm_check"),
            {"id": "frontend_wasm_check", "status": ERROR, "failure_code": "PREFLIGHT_MISSING"},
        ),
        {
            "id": "animation_debug_runtime",
            "status": PASS if backend_report is not None else FAIL,
            "failure_code": None if backend_report is not None else "ANIMATION_DIAGNOSTIC_REPORT_INVALID",
            "report_path": str(backend_report_path),
            "server_evidence_present": backend_report is not None and bool(backend_report.get("server_evidence")),
        },
    ]
    fault = fault_verification(requested_fault)
    status = overall_status(matrix, supplemental, preflight, fault)
    failure_codes = sorted(
        {
            item.get("failure_code")
            for collection in (preflight, matrix, supplemental, [fault])
            for item in collection
            if item.get("status") not in {PASS, WARN} and item.get("failure_code")
        }
    )
    runtime_failure_layer = backend_report.get("failure_layer") if backend_report else None
    report = {
        "schema_version": SCHEMA_VERSION,
        "milestone": "B0-M0",
        "command": "make animation-debug",
        "status": status,
        "pass": status == PASS,
        "failure_layer": runtime_failure_layer or ("CLIENT_E2E_UNAVAILABLE" if scenarios["scenario_c_hummingbird_client"]["status"] == UNAVAILABLE else None),
        "failure_codes": failure_codes,
        "preflight": preflight,
        "matrix": matrix,
        "supplemental": supplemental,
        "fault_injection": fault,
        "evidence": {
            "backend_report_path": str(backend_report_path),
            "backend_report": backend_report,
            "scenario_ids": sorted(scenarios),
        },
        "decision_table": decision_table(),
        "runtime_proof_boundary": [
            "A and D-G are focused native/backend harnesses; they do not prove browser presentation.",
            "B is PASS only when the real headless Hummingbird report contains selected server samples and progression evidence.",
            "C stays UNAVAILABLE without a real Tauri/browser WebRTC client scenario; frontend unit/WASM checks do not substitute for it.",
            "Fault verification stays UNAVAILABLE without a harness that changes evidence and exercises the normal classifier.",
        ],
        "report_path": str(output),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report, 0 if status == PASS else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "target/animation-debug-report.json",
        help="JSON report path (default: target/animation-debug-report.json)",
    )
    parser.add_argument(
        "--fault",
        help="Name a requested fault; reported UNAVAILABLE until a real fault harness exists",
    )
    args = parser.parse_args()
    output = args.output if args.output.is_absolute() else ROOT / args.output
    try:
        report, status = build_report(output, args.fault)
    except (OSError, ValueError, TypeError) as error:
        print(f"animation-debug failed before report completion: {error}", file=sys.stderr)
        return 1
    print(f"animation-debug: {report['status']} ({len(report['failure_codes'])} failure codes)")
    print(f"report: {output}")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
