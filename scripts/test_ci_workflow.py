#!/usr/bin/env python3
"""Structural invariant tests for GitHub Actions workflows in .github/workflows/."""

from __future__ import annotations

import copy
from pathlib import Path
import re
import tomllib
import unittest
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
RUST_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "rust.yml"
CAPABILITY_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "capability-gates.yml"


class YamlParseError(ValueError):
    """Raised when a workflow YAML file violates expected YAML subset rules."""


def _strip_comment(line: str) -> str:
    in_single = False
    in_double = False
    escaped = False
    for idx, ch in enumerate(line):
        if escaped:
            escaped = False
            continue
        if ch == "\\" and in_double:
            escaped = True
            continue
        if ch == "'" and not in_double:
            in_single = not in_single
        elif ch == '"' and not in_single:
            in_double = not in_double
        elif ch == "#" and not in_single and not in_double:
            if idx == 0 or line[idx - 1].isspace():
                return line[:idx]
    return line


def _parse_scalar(text: str) -> Any:
    raw = text.strip()
    if raw == "":
        return None
    if raw == "true":
        return True
    if raw == "false":
        return False
    if (raw.startswith('"') and raw.endswith('"')) or (
        raw.startswith("'") and raw.endswith("'")
    ):
        return raw[1:-1]
    if raw.startswith("[") and raw.endswith("]"):
        inner = raw[1:-1].strip()
        if not inner:
            return []
        return [_parse_scalar(part) for part in inner.split(",")]
    return raw


def _split_key_value(content: str) -> tuple[str, str | None] | None:
    in_single = False
    in_double = False
    for idx, ch in enumerate(content):
        if ch == "'" and not in_double:
            in_single = not in_single
        elif ch == '"' and not in_single:
            in_double = not in_double
        elif ch == ":" and not in_single and not in_double:
            if idx + 1 == len(content) or content[idx + 1].isspace():
                key = content[:idx].strip().strip("\"'")
                rest = content[idx + 1 :].strip()
                return key, (rest if rest != "" else None)
    return None


def parse_yaml_subset(raw_text: str) -> Any:
    """Parse indentation-based YAML mappings, lists, scalars, and block strings."""
    lines = raw_text.splitlines()
    tokens: list[tuple[int, str]] = []
    idx = 0
    while idx < len(lines):
        raw_line = lines[idx]
        if "\t" in raw_line[: len(raw_line) - len(raw_line.lstrip(" \t"))]:
            raise YamlParseError(f"line {idx + 1}: tabs are invalid in YAML indentation")
        cleaned = _strip_comment(raw_line).rstrip()
        if not cleaned.strip():
            idx += 1
            continue
        indent = len(cleaned) - len(cleaned.lstrip(" "))
        content = cleaned[indent:]
        tokens.append((indent, content))
        idx += 1

    # Re-parse with block-scalar awareness directly over lines.
    def _next_nonempty(start: int, min_indent: int) -> int | None:
        pos = start
        while pos < len(lines):
            cleaned = _strip_comment(lines[pos]).rstrip()
            if cleaned.strip():
                indent = len(cleaned) - len(cleaned.lstrip(" "))
                return pos if indent >= min_indent else None
            pos += 1
        return None

    def _collect_block_scalar(start: int, parent_indent: int) -> tuple[str, int]:
        collected: list[str] = []
        pos = start
        block_indent: int | None = None
        while pos < len(lines):
            line = lines[pos]
            if not line.strip():
                collected.append("")
                pos += 1
                continue
            indent = len(line) - len(line.lstrip(" "))
            if indent <= parent_indent:
                break
            if block_indent is None:
                block_indent = indent
            collected.append(line[min(block_indent, len(line)) :])
            pos += 1
        return "\n".join(collected), pos

    def _line_info(pos: int) -> tuple[int, str] | None:
        while pos < len(lines):
            cleaned = _strip_comment(lines[pos]).rstrip()
            if cleaned.strip():
                indent = len(cleaned) - len(cleaned.lstrip(" "))
                return indent, cleaned[indent:]
            pos += 1
        return None

    def _skip_empty(pos: int) -> int:
        while pos < len(lines):
            cleaned = _strip_comment(lines[pos]).rstrip()
            if cleaned.strip():
                return pos
            pos += 1
        return pos

    def _parse_node(pos: int, expected_indent: int) -> tuple[Any, int]:
        pos = _skip_empty(pos)
        if pos >= len(lines):
            return None, pos
        info = _line_info(pos)
        assert info is not None
        indent, content = info
        if indent != expected_indent:
            raise YamlParseError(
                f"line {pos + 1}: unexpected indent {indent}, expected {expected_indent}"
            )
        if content.startswith("- "):
            return _parse_list(pos, expected_indent)
        return _parse_dict(pos, expected_indent)

    def _parse_list(pos: int, base_indent: int) -> tuple[list[Any], int]:
        items: list[Any] = []
        while True:
            pos = _skip_empty(pos)
            if pos >= len(lines):
                break
            info = _line_info(pos)
            if info is None:
                break
            indent, content = info
            if indent < base_indent:
                break
            if indent > base_indent:
                raise YamlParseError(f"line {pos + 1}: unexpected list indent {indent}")
            if not (content == "-" or content.startswith("- ")):
                break

            rest = content[2:].strip() if content.startswith("- ") else ""
            item_indent = base_indent + 2
            if not rest:
                nxt = _next_nonempty(pos + 1, item_indent)
                if nxt is None:
                    items.append(None)
                    pos += 1
                else:
                    nxt_info = _line_info(nxt)
                    assert nxt_info is not None
                    val, pos = _parse_node(nxt, nxt_info[0])
                    items.append(val)
                continue

            kv = _split_key_value(rest)
            if kv is None:
                items.append(_parse_scalar(rest))
                pos += 1
                continue

            # Inline mapping item starting on the `- ` line (e.g. `- name: foo`).
            key, inline_val = kv
            entry: dict[str, Any] = {}
            if inline_val == ">":
                raise YamlParseError("folded run scalars are outside the reviewed YAML subset")
            if inline_val == "|":
                block_text, pos = _collect_block_scalar(pos + 1, item_indent)
                entry[key] = block_text
            elif inline_val is not None:
                entry[key] = _parse_scalar(inline_val)
                pos += 1
            else:
                nxt = _next_nonempty(pos + 1, item_indent + 1)
                if nxt is None:
                    entry[key] = None
                    pos += 1
                else:
                    nxt_info = _line_info(nxt)
                    assert nxt_info is not None
                    val, pos = _parse_node(nxt, nxt_info[0])
                    entry[key] = val

            nxt = _skip_empty(pos)
            if nxt < len(lines):
                nxt_info = _line_info(nxt)
                if nxt_info is not None and nxt_info[0] == item_indent and not nxt_info[1].startswith("- "):
                    more, pos = _parse_dict(nxt, item_indent)
                    if set(entry) & set(more):
                        raise YamlParseError("duplicate list mapping key")
                    entry.update(more)
            items.append(entry)
        return items, pos

    def _parse_dict(pos: int, base_indent: int) -> tuple[dict[str, Any], int]:
        mapping: dict[str, Any] = {}
        while True:
            pos = _skip_empty(pos)
            if pos >= len(lines):
                break
            info = _line_info(pos)
            if info is None:
                break
            indent, content = info
            if indent < base_indent:
                break
            if indent > base_indent:
                raise YamlParseError(f"line {pos + 1}: unexpected mapping indent {indent}")
            if content.startswith("- "):
                break
            kv = _split_key_value(content)
            if kv is None:
                raise YamlParseError(f"line {pos + 1}: expected key: value, got {content!r}")
            key, inline_val = kv
            if key in mapping:
                raise YamlParseError(f"duplicate YAML key {key!r}")
            if inline_val == ">":
                raise YamlParseError("folded run scalars are outside the reviewed YAML subset")
            if inline_val == "|":
                block_text, pos = _collect_block_scalar(pos + 1, base_indent)
                mapping[key] = block_text
            elif inline_val is not None:
                mapping[key] = _parse_scalar(inline_val)
                pos += 1
            else:
                nxt = _next_nonempty(pos + 1, base_indent + 1)
                if nxt is None:
                    mapping[key] = None
                    pos += 1
                else:
                    nxt_info = _line_info(nxt)
                    assert nxt_info is not None
                    val, pos = _parse_node(nxt, nxt_info[0])
                    mapping[key] = val
        return mapping, pos

    if not tokens:
        raise YamlParseError("empty YAML document")
    parsed, end = _parse_node(0, tokens[0][0])
    if _skip_empty(end) != len(lines):
        raise YamlParseError("unparsed workflow content")
    if not isinstance(parsed, dict):
        raise YamlParseError("workflow root must be a YAML mapping")
    return parsed


def _walk_keys(node: Any, key_name: str, path: str = "$") -> list[tuple[str, Any]]:
    hits: list[tuple[str, Any]] = []
    if isinstance(node, dict):
        for k, v in node.items():
            sub_path = f"{path}.{k}"
            if k == key_name:
                hits.append((sub_path, v))
            hits.extend(_walk_keys(v, key_name, sub_path))
    elif isinstance(node, list):
        for idx, item in enumerate(node):
            hits.extend(_walk_keys(item, key_name, f"{path}[{idx}]"))
    return hits


def _verified_workspace_test_targets(root: Path) -> dict[str, set[str]]:
    targets: dict[str, set[str]] = {}
    root_cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = root_cargo["workspace"]["members"]
    for member in ["."] + list(members):
        manifest_path = (root / member / "Cargo.toml").resolve()
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        pkg_name = manifest["package"]["name"]
        declared = {entry["name"] for entry in manifest.get("test", [])}
        tests_dir = manifest_path.parent / "tests"
        if manifest.get("package", {}).get("autotests", True) and tests_dir.is_dir():
            for rs_file in tests_dir.glob("*.rs"):
                declared.add(rs_file.stem)
        targets[pkg_name] = declared
    return targets


# The reviewed workflow contract is deliberately bounded. These literals are
# independent of the YAML loaded by the tests: a workflow change must update
# the explicit reviewed contract rather than accidentally passing a substring.
APPROVED_STEPS = {'checkout_bevy_openusd': {'name': 'Checkout bevy_openusd',
                           'uses': 'actions/checkout@v4',
                           'with': {'path': 'bevy_openusd', 'persist-credentials': False}},
 'setup_python': {'name': 'Setup Python', 'uses': 'actions/setup-python@v5', 'with': {'python-version': '3.12'}},
 'verify_python_3_11': {'name': 'Verify Python >= 3.11', 'run': 'python3 --version'},
 'install_rust_toolchain': {'name': 'Install Rust toolchain',
                            'uses': 'dtolnay/rust-toolchain@stable',
                            'with': {'components': 'rustfmt, clippy'}},
 'install_bevy_and_gstreamer_linux_runtime_deps': {'name': 'Install Bevy and GStreamer Linux runtime deps',
                                                   'run': 'sudo apt-get update\n'
                                                          'sudo apt-get install -y \\\n'
                                                          '  pkg-config \\\n'
                                                          '  libasound2-dev \\\n'
                                                          '  libudev-dev \\\n'
                                                          '  libwayland-dev \\\n'
                                                          '  libxkbcommon-dev \\\n'
                                                          '  libx11-dev \\\n'
                                                          '  libxcursor-dev \\\n'
                                                          '  libxi-dev \\\n'
                                                          '  libxrandr-dev \\\n'
                                                          '  libgstreamer1.0-dev \\\n'
                                                          '  libgstreamer-plugins-base1.0-dev \\\n'
                                                          '  libgstreamer-plugins-bad1.0-dev \\\n'
                                                          '  gstreamer1.0-plugins-base \\\n'
                                                          '  gstreamer1.0-plugins-good \\\n'
                                                          '  gstreamer1.0-plugins-bad \\\n'
                                                          '  gstreamer1.0-plugins-ugly \\\n'
                                                          '  gstreamer1.0-libav \\\n'
                                                          '  gstreamer1.0-nice'},
 'test_architecture_guard': {'name': 'Test architecture guard',
                             'run': 'python3 -B scripts/check_test_architecture.py --root . --json'},
 'test_architecture_guard_unit_self_tests': {'name': 'Test architecture guard unit self-tests',
                                             'run': 'python3 -B -m unittest '
                                                    'scripts/test_check_test_architecture.py -v'},
 'parity_and_ci_workflow_self_tests': {'name': 'Parity and CI workflow self-tests',
                                       'run': 'python3 -B -m unittest scripts/test_test_enumeration_parity.py '
                                              'scripts/test_ci_workflow.py -v'},
 'cargo_fmt': {'name': 'cargo fmt', 'run': 'cargo fmt --all -- --check'},
 'cargo_check_workspace': {'name': 'cargo check workspace',
                           'run': 'cargo check --workspace --all-targets --locked'},
 'cargo_build_workspace': {'name': 'cargo build workspace',
                           'run': 'cargo build --workspace --all-targets --locked'},
 'test_enumeration_parity_check': {'name': 'Test enumeration parity check',
                                   'run': 'python3 -B scripts/check_test_enumeration_parity.py --root . --json'},
 'cargo_test_workspace': {'name': 'cargo test workspace', 'run': 'cargo test --workspace --locked'},
 'contract_and_architecture_explicit_targets': {'name': 'Contract and architecture explicit targets',
                                                'run': 'cargo test --locked --test test_architecture\n'
                                                       'cargo test --locked -p project_support -p usd_support -p '
                                                       'viewport_support\n'
                                                       'cargo test --locked -p project_protocol --test contracts\n'
                                                       'cargo test --locked -p viewport_protocol --test bim '
                                                       '--test compatibility --test hierarchy --test '
                                                       'relocated_contracts --test renderer_configuration --test '
                                                       'session_wire --test viewer_settings\n'
                                                       'cargo test --locked -p viewport_streaming --test '
                                                       'relocated_contracts\n'
                                                       'cargo test --locked -p usd_semantic --test identity\n'
                                                       'cargo test --locked -p usd_diff --test diff_matrix\n'
                                                       'cargo test --locked -p usd_git --test branch_switch '
                                                       '--test repository --test tracked_paths\n'
                                                       'cargo test --locked --test live_projection --test '
                                                       'rapier_bridge --test git_materialization --test '
                                                       'point_instancer_correctness'},
 'install_rust_toolchain_step_17': {'name': 'Install Rust toolchain', 'uses': 'dtolnay/rust-toolchain@stable'},
 'cargo_check_solari_feature': {'name': 'cargo check solari feature',
                                'run': 'cargo check --workspace --all-targets --features solari --locked'},
 'test_enumeration_parity_check_solari': {'name': 'Test enumeration parity check (solari)',
                                          'run': 'python3 -B scripts/check_test_enumeration_parity.py --root . '
                                                 '--include-solari --json'},
 'run_cpu_safe_solari_unit_tests': {'name': 'Run CPU-safe solari unit tests',
                                    'run': 'cargo test -p usdview --lib --features solari --locked '
                                           'viewport::rendering::solari'},
 'require_provisioned_gstreamer_msvc_sdk': {'name': 'Require provisioned GStreamer MSVC SDK',
                                            'shell': 'pwsh',
                                            'run': 'if (-not $env:PKG_CONFIG -or -not (Test-Path -PathType Leaf '
                                                   '$env:PKG_CONFIG)) {\n'
                                                   '  throw "Provisioned Windows runner requires PKG_CONFIG '
                                                   'pointing to GStreamer pkg-config.exe."\n'
                                                   '}\n'
                                                   '& $env:PKG_CONFIG --exists gstreamer-1.0 gstreamer-base-1.0 '
                                                   'gstreamer-app-1.0 gstreamer-video-1.0 gstreamer-webrtc-1.0 '
                                                   'gstreamer-sdp-1.0 glib-2.0 gobject-2.0\n'
                                                   'if ($LASTEXITCODE -ne 0) { throw "GStreamer development '
                                                   'modules unavailable." }\n'
                                                   '& $env:PKG_CONFIG --modversion gstreamer-1.0\n'
                                                   'if ($LASTEXITCODE -ne 0) { throw "GStreamer version probe '
                                                   'failed." }'},
 'windows_enumeration_parity_and_exact_windows_only_execution': {'name': 'Windows enumeration parity and exact '
                                                                         'Windows-only execution',
                                                                 'run': 'python -B '
                                                                        'scripts/check_test_enumeration_parity.py '
                                                                        '--root . --platform windows '
                                                                        '--execute-windows --json'},
 'install_gstreamer_on_macos': {'name': 'Install GStreamer on macOS', 'run': 'brew install pkg-config gstreamer'},
 'macos_enumeration_parity_check': {'name': 'macOS enumeration parity check',
                                    'run': 'python -B scripts/check_test_enumeration_parity.py --root . '
                                           '--platform macos --json'},
 'cargo_test_workspace_on_macos': {'name': 'cargo test workspace on macOS',
                                   'run': 'cargo test --workspace --locked'},
 'cargo_check_dlss_feature': {'name': 'cargo check dlss feature',
                              'run': 'cargo check --workspace --all-targets --features dlss --locked'},
 'run_dlss_feature_tests_on_rtx_gpu_runner': {'name': 'Run DLSS feature tests on RTX GPU runner',
                                              'run': 'cargo test -p usdview --lib --features dlss --locked '
                                                     'viewport::rendering::sampling::dlss'},
 'run_external_fixture_and_profile_ignored_tests': {'name': 'Run external-fixture and profile ignored tests',
                                                    'run': 'cargo test --locked -p usd_bevy --lib -- --ignored\n'
                                                           'cargo test --locked -p usd_semantic --lib -- --ignored\n'
                                                           'cargo test --locked -p usdview --lib '
                                                           'classification_real_fixture -- --ignored\n'
                                                           'cargo test --locked -p usdview --lib profile_i1_7_ '
                                                           '-- --ignored'}}

APPROVED_JOBS = {'build-test': {'name': 'build + test',
                'runs-on': 'ubuntu-24.04',
                'steps': ['checkout_bevy_openusd',
                          'setup_python',
                          'verify_python_3_11',
                          'install_rust_toolchain',
                          'install_bevy_and_gstreamer_linux_runtime_deps',
                          'test_architecture_guard',
                          'test_architecture_guard_unit_self_tests',
                          'parity_and_ci_workflow_self_tests',
                          'cargo_fmt',
                          'cargo_check_workspace',
                          'cargo_build_workspace',
                          'test_enumeration_parity_check',
                          'cargo_test_workspace',
                          'contract_and_architecture_explicit_targets']},
 'solari-feature-gate': {'name': 'solari feature gate',
                         'runs-on': 'ubuntu-24.04',
                         'steps': ['checkout_bevy_openusd',
                                   'setup_python',
                                   'install_rust_toolchain_step_17',
                                   'install_bevy_and_gstreamer_linux_runtime_deps',
                                   'cargo_check_solari_feature',
                                   'test_enumeration_parity_check_solari',
                                   'run_cpu_safe_solari_unit_tests']},
 'windows-os-gate': {'name': 'windows-os-gate',
                     'if': "github.event.inputs.gate == 'windows-os-gate'",
                     'runs-on': ['self-hosted', 'windows', 'x64', 'gstreamer-msvc'],
                     'env': {'CARGO_BUILD_JOBS': '1'},
                     'steps': ['checkout_bevy_openusd',
                               'setup_python',
                               'install_rust_toolchain_step_17',
                               'require_provisioned_gstreamer_msvc_sdk',
                               'windows_enumeration_parity_and_exact_windows_only_execution']},
 'macos-external-assets-gate': {'name': 'macos-external-assets-gate',
                                'if': "github.event.inputs.gate == 'macos-external-assets-gate'",
                                'runs-on': 'macos-15',
                                'steps': ['checkout_bevy_openusd',
                                          'setup_python',
                                          'install_rust_toolchain_step_17',
                                          'install_gstreamer_on_macos',
                                          'macos_enumeration_parity_check',
                                          'cargo_test_workspace_on_macos']},
 'dlss-gpu-gate': {'name': 'dlss-gpu-gate',
                   'if': "github.event.inputs.gate == 'dlss-gpu-gate'",
                   'runs-on': ['self-hosted', 'nvidia-rtx', 'gpu'],
                   'steps': ['checkout_bevy_openusd',
                             'cargo_check_dlss_feature',
                             'run_dlss_feature_tests_on_rtx_gpu_runner']},
 'external-revit-and-profile-ignored-gate': {'name': 'external-revit-and-profile-ignored-gate',
                                             'if': 'github.event.inputs.gate == '
                                                   "'external-revit-and-profile-ignored-gate'",
                                             'runs-on': ['self-hosted', 'linux', 'revit-corpus'],
                                             'steps': ['checkout_bevy_openusd',
                                                       'run_external_fixture_and_profile_ignored_tests']}}

def _job_contract_errors(name: str, job: Any) -> list[str]:
    if not isinstance(job, dict):
        return [f"missing job {name}"]
    expected = APPROVED_JOBS[name]
    errors = []
    if {k: v for k, v in job.items() if k != "steps"} != {k: v for k, v in expected.items() if k != "steps"}:
        errors.append(f"{name}: runner, selector or job execution configuration differs from reviewed contract")
    actual = job.get("steps")
    if not isinstance(actual, list):
        return errors + [f"{name}: missing executable steps"]
    if len(actual) != len(expected["steps"]):
        errors.append(f"{name}: step count differs from reviewed contract")
    for i, key in enumerate(expected["steps"]):
        contract = APPROVED_STEPS[key]
        step = actual[i] if i < len(actual) else None
        if isinstance(step, dict):
            step = dict(step)
            if isinstance(step.get("run"), str):
                step["run"] = step["run"].strip()
        if step != contract:
            errors.append(f"{name}: missing or changed executable step {contract['name']}: {contract.get('run', contract.get('uses'))}")
    return errors


def _common_workflow_errors(workflow: dict[str, Any]) -> list[str]:
    errors = []
    if workflow.get("defaults") != {"run": {"working-directory": "bevy_openusd"}}:
        errors.append("workflow working-directory changed")
    if workflow.get("permissions") != {"contents": "read"}:
        errors.append("workflow permissions changed")
    if workflow.get("env") != {"CARGO_TERM_COLOR": "always"}:
        errors.append("workflow environment differs from reviewed contract")
    for path, val in _walk_keys(workflow, "continue-on-error"):
        errors.append(f"forbidden continue-on-error at {path} ({val!r})")
    return errors


def validate_rust_workflow(workflow: dict[str, Any], *, workspace_root: Path = REPO_ROOT) -> list[str]:
    errors = _common_workflow_errors(workflow)
    on_block = workflow.get("on")
    expected_branches = ["main", "develop", "develop/refactoring"]
    if not isinstance(on_block, dict):
        return errors + ["missing top-level 'on' trigger block"]
    for trigger_name in ("push", "pull_request"):
        if on_block.get(trigger_name) != {"branches": expected_branches}:
            errors.append(f"on.{trigger_name}.branches must be {expected_branches}")
    if set(on_block) != {"push", "pull_request", "workflow_dispatch"} or on_block.get("workflow_dispatch") is not None:
        errors.append("primary workflow dispatch/trigger contract changed")
    for path, val in _walk_keys(workflow, "if"):
        errors.append(f"forbidden conditional skip 'if' in rust.yml at {path} ({val!r})")
    jobs = workflow.get("jobs")
    if not isinstance(jobs, dict):
        return errors + ["missing 'jobs' mapping"]
    required_jobs = {"build-test", "solari-feature-gate"}
    if set(jobs) != required_jobs:
        errors.append("primary job inventory differs from reviewed contract")
    for name in sorted(required_jobs):
        errors.extend(_job_contract_errors(name, jobs.get(name)))
    # Keep the independent manifest existence check even for malformed/extra
    # steps, so phantom-target diagnostics remain actionable.
    declared_targets = _verified_workspace_test_targets(workspace_root)
    for job in jobs.values():
        if not isinstance(job, dict) or not isinstance(job.get("steps"), list):
            continue
        for step in job["steps"]:
            if not isinstance(step, dict):
                continue
            for line in str(step.get("run", "")).splitlines():
                line = line.strip()
                if not line.startswith("cargo test"):
                    continue
                pkg_match = re.search(r"-p\s+([a-zA-Z0-9_-]+)", line)
                pkg = pkg_match.group(1) if pkg_match else "usdview"
                for target in re.findall(r"--test\s+([a-zA-Z0-9_-]+)", line):
                    if target not in declared_targets.get(pkg, set()):
                        errors.append(f"unknown integration target {target!r} for package {pkg!r}")
    return errors


def validate_capability_workflow(workflow: dict[str, Any]) -> list[str]:
    errors = _common_workflow_errors(workflow)
    gates = ["windows-os-gate", "macos-external-assets-gate", "dlss-gpu-gate", "external-revit-and-profile-ignored-gate"]
    dispatch = {"workflow_dispatch": {"inputs": {"gate": {
        "description": "Capability-specific gate to execute", "required": True,
        "type": "choice", "options": gates}}}}
    if workflow.get("on") != dispatch:
        errors.append("capability dispatch choice/selector contract changed")
    jobs = workflow.get("jobs")
    if not isinstance(jobs, dict):
        return errors + ["missing capability jobs"]
    if set(jobs) != set(gates):
        errors.append("capability job inventory differs from reviewed contract")
    for gate in gates:
        errors.extend(_job_contract_errors(gate, jobs.get(gate)))
    return errors


class TestCiWorkflows(unittest.TestCase):
    def test_rust_workflow_uses_lf_and_passes_all_invariants(self) -> None:
        raw_bytes = RUST_WORKFLOW.read_bytes()
        self.assertNotIn(b"\r\n", raw_bytes, "rust.yml must use LF line endings")
        parsed = parse_yaml_subset(raw_bytes.decode("utf-8"))
        self.assertEqual(validate_rust_workflow(parsed), [])

    def test_capability_gates_workflow_uses_lf_and_passes_invariants(self) -> None:
        raw_bytes = CAPABILITY_WORKFLOW.read_bytes()
        self.assertNotIn(b"\r\n", raw_bytes, "capability-gates.yml must use LF line endings")
        parsed = parse_yaml_subset(raw_bytes.decode("utf-8"))
        self.assertEqual(validate_capability_workflow(parsed), [])

    def test_rejects_missing_branch_continue_on_error_and_if_skip(self) -> None:
        base = parse_yaml_subset(RUST_WORKFLOW.read_text(encoding="utf-8"))

        missing_branch = copy.deepcopy(base)
        missing_branch["on"]["pull_request"]["branches"] = ["main", "develop"]
        self.assertTrue(
            any("pull_request.branches" in err for err in validate_rust_workflow(missing_branch))
        )

        with_continue = copy.deepcopy(base)
        with_continue["jobs"]["build-test"]["steps"][0]["continue-on-error"] = True
        self.assertTrue(
            any("continue-on-error" in err for err in validate_rust_workflow(with_continue))
        )

        with_skip = copy.deepcopy(base)
        with_skip["jobs"]["solari-feature-gate"]["if"] = "false"
        self.assertTrue(
            any("conditional skip" in err for err in validate_rust_workflow(with_skip))
        )

    def test_rejects_removed_gate_or_phantom_cargo_target(self) -> None:
        base = parse_yaml_subset(RUST_WORKFLOW.read_text(encoding="utf-8"))

        no_guard = copy.deepcopy(base)
        for step in no_guard["jobs"]["build-test"]["steps"]:
            if isinstance(step, dict) and "check_test_architecture.py" in str(step.get("run", "")):
                step["run"] = "echo skipped"
        self.assertTrue(
            any("check_test_architecture.py" in err for err in validate_rust_workflow(no_guard))
        )

        phantom_target = copy.deepcopy(base)
        phantom_target["jobs"]["build-test"]["steps"].append(
            {"name": "phantom", "run": "cargo test --locked -p usd_semantic --test nonexistent_suite"}
        )
        self.assertTrue(
            any("unknown integration target 'nonexistent_suite'" in err for err in validate_rust_workflow(phantom_target))
        )

    def test_rejects_noop_masked_and_deleted_required_commands(self) -> None:
        base = parse_yaml_subset(RUST_WORKFLOW.read_text(encoding="utf-8"))
        for mutation in ("echo", "comment", "mask", "remove_viewport", "remove_git", "remove_root", "early_exit"):
            with self.subTest(mutation=mutation):
                broken = copy.deepcopy(base)
                steps = broken["jobs"]["build-test"]["steps"]
                if mutation in {"echo", "comment", "mask"}:
                    step = next(x for x in steps if x.get("name") == "Test architecture guard")
                    cmd = step["run"]
                    step["run"] = {"echo": f'echo "{cmd}"', "comment": f"# {cmd}", "mask": cmd + " || true"}[mutation]
                else:
                    step = next(x for x in steps if x.get("name") == "Contract and architecture explicit targets")
                    if mutation == "early_exit":
                        step["run"] = "exit 0\n" + step["run"]
                    else:
                        needle = {"remove_viewport": "-p viewport_protocol", "remove_git": "-p usd_git", "remove_root": "--test live_projection"}[mutation]
                        step["run"] = "\n".join(line for line in step["run"].splitlines() if needle not in line)
                self.assertTrue(validate_rust_workflow(broken))
        self.assertEqual(validate_rust_workflow(base), [])

    def test_rejects_capability_noop_wrong_selector_missing_prerequisite(self) -> None:
        base = parse_yaml_subset(CAPABILITY_WORKFLOW.read_text(encoding="utf-8"))
        for mutation in ("empty", "echo", "selector", "choice", "runner", "sdk", "skip", "wrong_filter"):
            with self.subTest(mutation=mutation):
                broken = copy.deepcopy(base)
                job = broken["jobs"]["windows-os-gate"]
                if mutation == "empty": job["steps"] = []
                elif mutation == "echo": job["steps"][-1]["run"] = 'echo "' + job["steps"][-1]["run"] + '"'
                elif mutation == "selector": job["if"] = "false"
                elif mutation == "choice": broken["on"]["workflow_dispatch"]["inputs"]["gate"]["options"].pop()
                elif mutation == "runner": job["runs-on"] = "windows-latest"
                elif mutation == "sdk": job["steps"].pop(-2)
                elif mutation == "skip": job["steps"][-1]["if"] = "false"
                else: job["steps"][-1]["run"] = "cargo test -p usdview --lib --locked project::source_closure"
                self.assertTrue(validate_capability_workflow(broken))
        self.assertEqual(validate_capability_workflow(base), [])


if __name__ == "__main__":
    unittest.main()
