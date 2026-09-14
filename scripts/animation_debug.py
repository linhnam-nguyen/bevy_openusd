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
import math
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
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
CLIENT_READY_TIMEOUT_SECONDS = 120
FRONTEND_SERVER_TIMEOUT_SECONDS = 60
CLIENT_MEASUREMENT_SECONDS = 4
ANIMATION_RUNTIME_TIMEOUT_SECONDS = 90

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
    {
        "id": "H",
        "description": "actual compact animation-debug protocol event size",
        "scenario": "scenario_h_protocol_message_size",
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


def finite_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


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
    backend_report: dict[str, Any] | None = None,
) -> str:
    matrix_pass = all(row.get("status") == PASS for row in matrix)
    supplemental_pass = all(check.get("status") == PASS for check in supplemental)
    preflight_pass = all(check.get("status") in {PASS, WARN} for check in preflight)
    fault_pass = fault_report.get("status") == PASS
    backend_pass = backend_report is None or backend_report.get("pass") is True
    return (
        PASS
        if matrix_pass and supplemental_pass and preflight_pass and fault_pass and backend_pass
        else FAIL
    )


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


def backend_animation_command(output: Path, fault: str | None = None) -> list[str]:
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
    ]
    if fault is not None:
        command.extend(["--animation-debug-fault", fault])
    command.append("assets/external/hummingbird.usdz")
    return command


def parse_backend_report(
    output: Path, command_check: dict[str, Any]
) -> dict[str, Any] | None:
    if not output.is_file():
        command_check["status"] = FAIL
        command_check["failure_code"] = "HEADLESS_RENDER_REPORT_MISSING"
        command_check["error"] = "backend diagnostic did not write its JSON report"
        return None
    try:
        report = json.loads(output.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        command_check["status"] = FAIL
        command_check["failure_code"] = "HEADLESS_RENDER_REPORT_INVALID"
        command_check["error"] = str(error)
        return None
    command_check["report_schema_version"] = report.get("schema_version")
    command_check["backend_report_status"] = report.get("pass")
    return report


def wait_for_port(host: str, port: int, timeout_seconds: int) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return True
        except OSError:
            time.sleep(0.25)
    return False


def wait_for_file(path: Path, timeout_seconds: int) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if path.is_file():
            return True
        time.sleep(0.25)
    return path.is_file()


def stop_processes(processes: Sequence[subprocess.Popen[Any]]) -> None:
    for process in reversed(processes):
        if process.poll() is not None:
            continue
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except (OSError, ProcessLookupError):
            process.terminate()
    for process in processes:
        if process.poll() is not None:
            continue
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except (OSError, ProcessLookupError):
                process.kill()
            process.wait(timeout=5)


def load_client_evidence(path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def run_backend_only(
    output: Path, fault: str | None = None
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.unlink(missing_ok=True)
    check_id = "fault_backend_render_command" if fault else "scenario_b_hummingbird_render_command"
    failure_code = "FAULT_BACKEND_COMMAND_FAILED" if fault else "HEADLESS_RENDER_COMMAND_FAILED"
    command_check = command_result(
        check_id,
        backend_animation_command(output, fault),
        ROOT,
        failure_code,
        timeout_seconds=240,
    )
    return command_check, parse_backend_report(output, command_check)


def run_backend_render(
    output: Path,
) -> tuple[dict[str, Any], dict[str, Any] | None, dict[str, Any], dict[str, Any] | None]:
    """Run B and C through the real native backend plus real Tauri/WebRTC client."""
    output.parent.mkdir(parents=True, exist_ok=True)
    output.unlink(missing_ok=True)
    runtime: dict[str, Any] = {
        "id": "animation_debug_runtime",
        "status": PASS,
        "failure_code": None,
        "scenario_code": "S12",
        "frontend_root": str(FRONTEND_ROOT),
        "frontend_server_started": False,
        "tauri_started": False,
        "backend_started": False,
        "client_ready": False,
        "client_evidence_present": False,
    }
    required_tools = ["cargo", "pnpm", "trunk", "cargo-tauri"]
    missing_tools = [tool for tool in required_tools if shutil.which(tool) is None]
    if missing_tools:
        runtime.update(
            status=UNAVAILABLE,
            failure_code="CLIENT_E2E_RUNTIME_UNAVAILABLE",
            reason=f"required runtime tools are missing: {', '.join(missing_tools)}",
        )
        command_check, report = run_backend_only(output)
        return command_check, report, runtime, None

    run_directory = Path(tempfile.mkdtemp(prefix="b0-m0-c5-", dir=ROOT / "target"))
    run_id = f"b0-m0-c5-{os.getpid()}-{int(time.time())}"
    markers = {
        "ready": run_directory / "ready",
        "start": run_directory / "measurement-start",
        "idle": run_directory / "measurement-idle",
        "complete": run_directory / "measurement-complete",
    }
    client_evidence_path = run_directory / "client-evidence.json"
    benchmark_env = os.environ.copy()
    # Trunk's --no-color parser rejects the common NO_COLOR=1 convention.
    # The diagnostic owns these child processes, so normalize only their env.
    benchmark_env.pop("NO_COLOR", None)
    benchmark_env.update(
        {
            "USDHUB_BENCHMARK_RUN_ID": run_id,
            "USDHUB_BENCHMARK_SCENARIO": "S12",
            "USDHUB_BENCHMARK_EVIDENCE": str(client_evidence_path),
            "USDHUB_BENCHMARK_SIGNALING_URL": "ws://127.0.0.1:8080",
            "USDHUB_BENCHMARK_READY_FILE": str(markers["ready"]),
            "USDHUB_BENCHMARK_MEASUREMENT_START_FILE": str(markers["start"]),
            "USDHUB_BENCHMARK_MEASUREMENT_IDLE_FILE": str(markers["idle"]),
            "USDHUB_BENCHMARK_MEASUREMENT_COMPLETE_FILE": str(markers["complete"]),
            "USDHUB_BENCHMARK_REQUESTED_WIDTH": "640",
            "USDHUB_BENCHMARK_REQUESTED_HEIGHT": "480",
            "USDHUB_BENCHMARK_REQUESTED_FPS": "30",
        }
    )
    processes: list[subprocess.Popen[Any]] = []
    command_check: dict[str, Any] = {
        "id": "scenario_b_hummingbird_render_command",
        "status": FAIL,
        "failure_code": "HEADLESS_RENDER_COMMAND_FAILED",
        "command": backend_animation_command(output),
        "cwd": str(ROOT),
    }
    report: dict[str, Any] | None = None
    client_evidence: dict[str, Any] | None = None
    try:
        try:
            frontend = subprocess.Popen(
                ["pnpm", "frontend:dev"],
                cwd=FRONTEND_ROOT,
                env=benchmark_env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            processes.append(frontend)
            runtime["frontend_server_started"] = wait_for_port(
                "127.0.0.1", 3000, FRONTEND_SERVER_TIMEOUT_SECONDS
            )
            if not runtime["frontend_server_started"]:
                raise RuntimeError("frontend dev server did not open port 3000")

            tauri = subprocess.Popen(
                ["pnpm", "tauri:dev:ui-e2e"],
                cwd=FRONTEND_ROOT,
                env=benchmark_env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            processes.append(tauri)
            runtime["tauri_started"] = True

            backend = subprocess.Popen(
                backend_animation_command(output),
                cwd=ROOT,
                env=benchmark_env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            processes.append(backend)
            runtime["backend_started"] = True

            runtime["client_ready"] = wait_for_file(
                markers["ready"], CLIENT_READY_TIMEOUT_SECONDS
            )
            if not runtime["client_ready"]:
                raise RuntimeError("Tauri/WebView client did not signal readiness")
            markers["start"].write_text("measurement-started\n", encoding="utf-8")
            time.sleep(CLIENT_MEASUREMENT_SECONDS)
            markers["complete"].write_text("measurement-complete\n", encoding="utf-8")

            try:
                backend.wait(timeout=ANIMATION_RUNTIME_TIMEOUT_SECONDS)
            except subprocess.TimeoutExpired:
                command_check["failure_code"] = "HEADLESS_RENDER_COMMAND_TIMEOUT"
            command_check["exit_code"] = backend.returncode
            command_check["status"] = PASS if backend.returncode == 0 else FAIL
            if not client_evidence_path.is_file():
                wait_for_file(client_evidence_path, 10)
            client_evidence = load_client_evidence(client_evidence_path)
            runtime["client_evidence_present"] = client_evidence is not None
        except (OSError, RuntimeError) as error:
            runtime.update(
                status=UNAVAILABLE,
                failure_code="CLIENT_E2E_RUNTIME_UNAVAILABLE",
                reason=str(error),
            )
            command_check["error"] = str(error)
    finally:
        stop_processes(processes)
        report = parse_backend_report(output, command_check)
        if runtime["status"] == UNAVAILABLE:
            fallback_check, report = run_backend_only(output)
            command_check = fallback_check
        if client_evidence is None:
            client_evidence = load_client_evidence(client_evidence_path)
            runtime["client_evidence_present"] = client_evidence is not None
        if runtime["status"] == PASS and not runtime["client_ready"]:
            runtime.update(
                status=UNAVAILABLE,
                failure_code="CLIENT_E2E_RUNTIME_UNAVAILABLE",
                reason="real client did not reach the measurement barrier",
            )
        shutil.rmtree(run_directory, ignore_errors=True)
    return command_check, report, runtime, client_evidence


def hummingbird_render_scenario(report: dict[str, Any] | None, command_check: dict[str, Any]) -> dict[str, Any]:
    if report is None:
        return {
            "id": "scenario_b_hummingbird_render",
            "status": FAIL,
            "failure_code": command_check.get("failure_code"),
        }
    server = report.get("server_evidence", {})
    pairwise = report.get("pairwise_render_mad_luma", {})
    thresholds = report.get("thresholds", {})
    t0 = server.get("t0") or {}
    t1 = server.get("t1") or {}
    repeat_mad = pairwise.get("hummingbird_t0_round_trip")
    repeat_threshold = thresholds.get("hummingbird_max_repeat_mad")
    motion_mad = pairwise.get("hummingbird_t0_t1")
    motion_threshold = thresholds.get("hummingbird_min_mad")
    required = {
        "stage_ready": t0.get("stage_ready") is True and t1.get("stage_ready") is True,
        "animated_prim_count": (report.get("animated_prim_count") or 0) > 0,
        "hash_t0_differs_from_t1": server.get("hash_t0_differs_from_t1") is True,
        "sequence_t1_after_t0": server.get("sequence_t1_after_t0") is True,
        "sequence_round_trip_after_t1": server.get("sequence_round_trip_after_t1") is True,
        "hummingbird_motion_mad_finite": finite_number(motion_mad),
        "hummingbird_motion_threshold_finite": finite_number(motion_threshold),
        "hummingbird_motion_mad_reaches_threshold": finite_number(motion_mad)
        and finite_number(motion_threshold)
        and motion_mad >= motion_threshold,
        "hummingbird_repeat_mad_finite": finite_number(repeat_mad),
        "hummingbird_repeat_threshold_finite": finite_number(repeat_threshold),
        "hummingbird_repeat_mad_within_threshold": finite_number(repeat_mad)
        and finite_number(repeat_threshold)
        and repeat_mad <= repeat_threshold,
    }
    status = PASS if all(required.values()) and command_check.get("exit_code") == 0 else FAIL
    return {
        "id": "scenario_b_hummingbird_render",
        "status": status,
        "failure_code": None if status == PASS else "HEADLESS_RENDER_EVIDENCE_INCOMPLETE",
        "evidence": {"required": required, "backend_report": report},
    }


def static_negative_scenario(
    semantic_check: dict[str, Any], report: dict[str, Any] | None, command_check: dict[str, Any]
) -> dict[str, Any]:
    if report is None:
        return {
            "id": "scenario_d_static",
            "status": FAIL,
            "failure_code": command_check.get("failure_code"),
        }
    server = report.get("server_evidence", {})
    pairwise = report.get("pairwise_render_mad_luma", {})
    thresholds = report.get("thresholds", {})
    static_t0 = server.get("static_t0") or {}
    static_t1 = server.get("static_t1") or {}
    static_mad = pairwise.get("static_t0_t1")
    static_threshold = thresholds.get("static_max_mad")
    required = {
        "semantic_static_test": semantic_check.get("status") == PASS,
        "static_samples_present": bool(static_t0) and bool(static_t1),
        "static_stage_ready": static_t0.get("stage_ready") is True and static_t1.get("stage_ready") is True,
        "static_animated_count_zero": static_t0.get("animated_prim_count") == 0
        and static_t1.get("animated_prim_count") == 0,
        "static_sequence_advances": server.get("static_sequence_advances") is True,
        "static_mad_finite": finite_number(static_mad),
        "static_threshold_finite": finite_number(static_threshold),
        "static_mad_within_threshold": finite_number(static_mad)
        and finite_number(static_threshold)
        and static_mad <= static_threshold,
        "backend_command_passed": command_check.get("exit_code") == 0,
    }
    status = PASS if all(required.values()) else FAIL
    return {
        "id": "scenario_d_static",
        "status": status,
        "failure_code": None if status == PASS else "STATIC_NEGATIVE_CONTROL_FAILED",
        "evidence": {"required": required, "semantic_check": semantic_check, "backend_report": report},
    }


def seek_round_trip_scenario(
    transform_check: dict[str, Any], report: dict[str, Any] | None, command_check: dict[str, Any]
) -> dict[str, Any]:
    if report is None:
        return {
            "id": "scenario_g_seek_round_trip",
            "status": FAIL,
            "failure_code": command_check.get("failure_code"),
        }
    server = report.get("server_evidence", {})
    pairwise = report.get("pairwise_render_mad_luma", {})
    thresholds = report.get("thresholds", {})
    repeat_mad = pairwise.get("hummingbird_t0_round_trip")
    repeat_threshold = thresholds.get("hummingbird_max_repeat_mad")
    required = {
        "semantic_transform_test": transform_check.get("status") == PASS,
        "selected_samples_present": all(
            isinstance(server.get(name), dict) for name in ("t0", "t1", "t0_round_trip")
        ),
        "sequence_t1_after_t0": server.get("sequence_t1_after_t0") is True,
        "sequence_round_trip_after_t1": server.get("sequence_round_trip_after_t1") is True,
        "transform_t0_differs_from_t1": server.get("transform_t0_differs_from_t1") is True,
        "transform_round_trip_matches_t0": server.get("transform_round_trip_matches_t0") is True,
        "repeat_mad_finite": finite_number(repeat_mad),
        "repeat_threshold_finite": finite_number(repeat_threshold),
        "repeat_mad_within_threshold": finite_number(repeat_mad)
        and finite_number(repeat_threshold)
        and repeat_mad <= repeat_threshold,
        "backend_command_passed": command_check.get("exit_code") == 0,
    }
    status = PASS if all(required.values()) else FAIL
    return {
        "id": "scenario_g_seek_round_trip",
        "status": status,
        "failure_code": None if status == PASS else "PAUSED_SEEK_ROUND_TRIP_FAILED",
        "evidence": {"required": required, "semantic_check": transform_check, "backend_report": report},
    }


def client_animation_scenario(
    report: dict[str, Any] | None,
    command_check: dict[str, Any],
    runtime: dict[str, Any],
    client_evidence: dict[str, Any] | None,
) -> dict[str, Any]:
    if runtime.get("status") == UNAVAILABLE:
        return unavailable(
            "scenario_c_hummingbird_client",
            runtime.get("reason", "real Tauri/WebView runtime was unavailable"),
            "CLIENT_E2E_RUNTIME_UNAVAILABLE",
        )
    if report is None:
        return {
            "id": "scenario_c_hummingbird_client",
            "status": FAIL,
            "failure_code": "CLIENT_E2E_EVIDENCE_INCOMPLETE",
            "evidence": {"runtime": runtime, "client_evidence": client_evidence},
        }

    samples = report.get("client_samples", [])
    summary = report.get("client_evidence", {})
    received_values = [sample.get("frames_received") for sample in samples if sample.get("frames_received") is not None]
    delivery_values = [sample.get("delivery_frames") for sample in samples if sample.get("delivery_frames") is not None]
    received_delta = (
        received_values[-1] - received_values[0] if len(received_values) >= 2 else 0
    )
    delivery_delta = (
        delivery_values[-1] - delivery_values[0] if len(delivery_values) >= 2 else 0
    )
    proofs = summary.get("proofs", [])
    presented_delta = summary.get("presented_delta", 0) or 0
    compositor_presented = presented_delta > 0 and "compositor" in proofs
    fallback_delivery = delivery_delta > 0 and any(
        proof in {"playback_quality", "rtp_decoded", "rtp_received"} for proof in proofs
    )
    decoded_delta = summary.get("decoded_delta", 0) or 0
    required = {
        "real_runtime_started": runtime.get("frontend_server_started")
        and runtime.get("tauri_started")
        and runtime.get("backend_started"),
        "client_snapshots": len(samples) > 0,
        "frames_received_delta": received_delta > 0,
        "frames_decoded_delta": decoded_delta > 0,
        "presentation_or_fallback_proof": compositor_presented or fallback_delivery,
        "measurement_evidence": client_evidence is not None,
    }
    status = PASS if all(required.values()) and command_check.get("exit_code") == 0 else FAIL
    if status == PASS:
        failure_code = None
    elif not required["client_snapshots"]:
        failure_code = "ENCODE_OR_WEBRTC_STALLED"
    elif not required["frames_decoded_delta"]:
        failure_code = "CLIENT_DECODE_STALLED"
    else:
        failure_code = "CLIENT_PRESENTATION_STALLED"
    return {
        "id": "scenario_c_hummingbird_client",
        "status": status,
        "failure_code": failure_code,
        "evidence": {
            "required": required,
            "received_delta": received_delta,
            "delivery_delta": delivery_delta,
            "presentation_source": "compositor_presented" if compositor_presented else "fallback_delivery_or_decode",
            "backend_client_evidence": summary,
            "client_evidence": client_evidence,
            "runtime": runtime,
        },
    }


def fault_verification(
    fault_name: str, expected_layer: str, report: dict[str, Any] | None
) -> dict[str, Any]:
    observed_layer = report.get("failure_layer") if report is not None else None
    generated_fault = report.get("diagnostic_fault") if report is not None else None
    status = (
        PASS
        if report is not None
        and report.get("pass") is False
        and generated_fault == fault_name
        and observed_layer == expected_layer
        else FAIL
    )
    return {
        "id": "fault_injection",
        "status": status,
        "failure_code": None if status == PASS else "FAULT_CLASSIFICATION_MISMATCH",
        "fault": fault_name,
        "expected": expected_layer,
        "observed": observed_layer,
        "generated_fault": generated_fault,
    }


def decision_table() -> list[dict[str, Any]]:
    return [
        {"when": "all A-H rows are PASS and the generated fault is classified", "decision": "eligible", "failure_code": None},
        {"when": "any required row is UNAVAILABLE or ERROR", "decision": "FAIL", "failure_code": "MATRIX_EVIDENCE_MISSING"},
        {"when": "the generated fault report is missing or misclassified", "decision": "FAIL", "failure_code": "FAULT_CLASSIFICATION_MISMATCH"},
        {"when": "preflight or supplemental check is FAIL", "decision": "FAIL", "failure_code": "CHECK_FAILED"},
    ]


def build_report(output: Path) -> tuple[dict[str, Any], int]:
    preflight = run_preflight()
    scenarios: dict[str, dict[str, Any]] = {}
    scenarios["scenario_a_hummingbird_stage"] = focused_test(
        "scenario_a_hummingbird_stage",
        "viewport::animation::tests::hummingbird_native_stage_evaluation_is_animated",
        "NATIVE_STAGE_EVALUATION_FAILED",
    )
    static_semantic_check = focused_test(
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
    transform_semantic_check = focused_test(
        "scenario_g_seek_round_trip",
        "viewport::animation::tests::hummingbird_paused_seek_round_trip_has_stable_transform_signature",
        "PAUSED_SEEK_ROUND_TRIP_FAILED",
    )

    backend_report_path = ROOT / "target/animation-debug-backend.json"
    render_command, backend_report, animation_runtime, client_evidence = run_backend_render(
        backend_report_path
    )
    scenarios["scenario_b_hummingbird_render"] = hummingbird_render_scenario(backend_report, render_command)
    scenarios["scenario_d_static"] = static_negative_scenario(
        static_semantic_check, backend_report, render_command
    )
    scenarios["scenario_g_seek_round_trip"] = seek_round_trip_scenario(
        transform_semantic_check, backend_report, render_command
    )
    scenarios["scenario_c_hummingbird_client"] = client_animation_scenario(
        backend_report,
        render_command,
        animation_runtime,
        client_evidence,
    )

    protocol_check = command_result(
        "protocol_debug_message_size",
        [
            "cargo",
            "test",
            "-p",
            "viewport_protocol",
            "animation_debug::tests::actual_debug_event_stays_below_one_kibibyte",
        ],
        ROOT,
        "PROTOCOL_DEBUG_MESSAGE_SIZE_FAILED",
    )
    scenarios["scenario_h_protocol_message_size"] = {
        "id": "scenario_h_protocol_message_size",
        "status": protocol_check["status"],
        "failure_code": protocol_check.get("failure_code"),
        "evidence": protocol_check,
    }

    matrix = [
        matrix_row(entry["id"], entry["description"], scenarios.get(entry["scenario"]))
        for entry in MATRIX
    ]
    supplemental = [
        protocol_check,
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
            "status": animation_runtime.get("status", FAIL),
            "failure_code": animation_runtime.get("failure_code"),
            "report_path": str(backend_report_path),
            "server_evidence_present": backend_report is not None and bool(backend_report.get("server_evidence")),
            "client_evidence_present": client_evidence is not None,
        },
    ]
    fault_report_path = ROOT / "target/animation-debug-fault.json"
    fault_command, fault_backend_report = run_backend_only(
        fault_report_path, fault="freeze-transform-evidence"
    )
    fault = fault_verification(
        "freeze_transform_evidence", "ANIMATION_EVALUATION_STATIC", fault_backend_report
    )
    fault["command"] = fault_command
    fault["report_path"] = str(fault_report_path)
    if fault_command.get("status") != PASS:
        fault["status"] = FAIL
        fault["failure_code"] = fault_command.get("failure_code", "FAULT_BACKEND_COMMAND_FAILED")
    status = overall_status(matrix, supplemental, preflight, fault, backend_report)
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
        "failure_layer": runtime_failure_layer
        or scenarios["scenario_c_hummingbird_client"].get("failure_code"),
        "failure_codes": failure_codes,
        "preflight": preflight,
        "matrix": matrix,
        "supplemental": supplemental,
        "fault_injection": fault,
        "evidence": {
            "backend_report_path": str(backend_report_path),
            "backend_report": backend_report,
            "client_evidence": client_evidence,
            "animation_runtime": animation_runtime,
            "scenario_ids": sorted(scenarios),
        },
        "decision_table": decision_table(),
        "runtime_proof_boundary": [
            "A and D-G are focused native/backend harnesses; they do not prove browser presentation.",
            "B is PASS only when the real headless Hummingbird report contains selected server samples and progression evidence.",
            "C is PASS only when the real Tauri/WebView client reports receive, decode, and compositor or labelled fallback delivery evidence; frontend unit/WASM checks do not substitute for it.",
            "H is backed by the actual serialized AnimationDebugServerSample protocol event and remains below one KiB without frame bytes.",
            "The automatic freeze-transform-evidence run must be classified by the normal report classifier as ANIMATION_EVALUATION_STATIC.",
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
    args = parser.parse_args()
    output = args.output if args.output.is_absolute() else ROOT / args.output
    try:
        report, status = build_report(output)
    except (OSError, ValueError, TypeError) as error:
        print(f"animation-debug failed before report completion: {error}", file=sys.stderr)
        return 1
    print(f"animation-debug: {report['status']} ({len(report['failure_codes'])} failure codes)")
    print(f"report: {output}")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
