#!/usr/bin/env python3
"""Target-qualified test parity gate for bevy_openusd.

The schema-1 manifest is retained solely as an aggregate cross-check. Canonical
CI requires the separately captured schema-2 manifest at the exact C1 commit;
flat stdout and schema-1 manifests cannot satisfy the canonical CLI.
"""

from __future__ import annotations

import argparse
from collections import Counter
import functools
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


SUPPORTED_PLATFORMS = frozenset({"windows", "linux", "macos"})
SUMMARY_LINE_RE = re.compile(r"^(\d+) tests?, (\d+) benchmarks?$")
BENCHMARK_SUFFIX = ": benchmark"
TEST_SUFFIX = ": test"


class BaselineError(ValueError):
    """Raised when the parity baseline manifest or list input is malformed."""


def detect_platform(platform_arg: str = "auto") -> str:
    if platform_arg != "auto":
        if platform_arg not in SUPPORTED_PLATFORMS:
            raise BaselineError(
                f"unsupported platform {platform_arg!r}; expected one of {sorted(SUPPORTED_PLATFORMS)}"
            )
        return platform_arg
    if sys.platform.startswith("win"):
        return "windows"
    if sys.platform == "darwin":
        return "macos"
    if sys.platform.startswith("linux"):
        return "linux"
    raise BaselineError(f"unsupported host platform {sys.platform}")


def _parse_entry_bucket(raw: Any, bucket_name: str) -> Counter[str]:
    if not isinstance(raw, list):
        raise BaselineError(f"baseline bucket {bucket_name!r} must be a list")
    counts: Counter[str] = Counter()
    for idx, item in enumerate(raw):
        if not isinstance(item, dict):
            raise BaselineError(f"baseline {bucket_name}[{idx}] must be an object")
        name = item.get("name")
        count = item.get("count")
        if not isinstance(name, str) or not name.strip() or name.endswith(TEST_SUFFIX):
            raise BaselineError(f"baseline {bucket_name}[{idx}].name is invalid: {name!r}")
        if isinstance(count, bool) or not isinstance(count, int) or count < 1:
            raise BaselineError(f"baseline {bucket_name}[{idx}].count must be >= 1: {count!r}")
        if name in counts:
            raise BaselineError(f"duplicate test identity {name!r} in {bucket_name}")
        counts[name] = count
    return counts


def load_baseline(baseline_path: Path) -> dict[str, Any]:
    data = read_json(baseline_path)

    if not isinstance(data, dict):
        raise BaselineError("baseline manifest must be a JSON object")
    if data.get("schema_version") != 1:
        raise BaselineError(f"unsupported baseline schema_version: {data.get('schema_version')!r}")

    for required_str in ("baseline_checkpoint", "baseline_commit", "command"):
        val = data.get(required_str)
        if not isinstance(val, str) or not val.strip():
            raise BaselineError(f"baseline missing non-empty {required_str!r}")

    common = _parse_entry_bucket(data.get("common_tests"), "common_tests")
    windows_only = _parse_entry_bucket(data.get("windows_only_tests"), "windows_only_tests")
    unix_only = _parse_entry_bucket(data.get("unix_only_tests"), "unix_only_tests")
    solari_feature = _parse_entry_bucket(data.get("solari_feature_tests"), "solari_feature_tests")

    for a_name, a_counts, b_name, b_counts in (
        ("common_tests", common, "windows_only_tests", windows_only),
        ("common_tests", common, "unix_only_tests", unix_only),
        ("common_tests", common, "solari_feature_tests", solari_feature),
        ("windows_only_tests", windows_only, "unix_only_tests", unix_only),
        ("windows_only_tests", windows_only, "solari_feature_tests", solari_feature),
        ("unix_only_tests", unix_only, "solari_feature_tests", solari_feature),
    ):
        overlap = set(a_counts) & set(b_counts)
        if overlap:
            raise BaselineError(
                f"baseline buckets {a_name} and {b_name} overlap on {sorted(overlap)[:5]}"
            )

    summary = data.get("summary")
    if not isinstance(summary, dict):
        raise BaselineError("baseline summary must be a JSON object")
    expected_summary = {
        "common_total_entries": sum(common.values()),
        "common_unique_identities": len(common),
        "windows_only_total_entries": sum(windows_only.values()),
        "windows_only_unique_identities": len(windows_only),
        "unix_only_total_entries": sum(unix_only.values()),
        "unix_only_unique_identities": len(unix_only),
        "solari_feature_total_entries": sum(solari_feature.values()),
        "solari_feature_unique_identities": len(solari_feature),
        "windows_default_total_entries": sum(common.values()) + sum(windows_only.values()),
        "windows_default_unique_identities": len(common) + len(windows_only),
        "unix_default_total_entries": sum(common.values()) + sum(unix_only.values()),
        "unix_default_unique_identities": len(common) + len(unix_only),
    }
    for key, expected_val in expected_summary.items():
        if summary.get(key) != expected_val:
            raise BaselineError(
                f"baseline summary.{key}={summary.get(key)!r} disagrees with entries ({expected_val})"
            )

    if expected_summary["windows_default_total_entries"] < 1:
        raise BaselineError("baseline cannot be empty")

    return {
        "baseline_checkpoint": data["baseline_checkpoint"],
        "baseline_commit": data["baseline_commit"],
        "command": data["command"],
        "summary": expected_summary,
        "common_tests": common,
        "windows_only_tests": windows_only,
        "unix_only_tests": unix_only,
        "solari_feature_tests": solari_feature,
    }


def expected_counts_for_platform(
    baseline: dict[str, Any],
    platform: str,
    *,
    include_solari: bool = False,
) -> Counter[str]:
    if platform not in SUPPORTED_PLATFORMS:
        raise BaselineError(f"unsupported platform {platform!r}")
    expected: Counter[str] = Counter(baseline["common_tests"])
    if platform == "windows":
        expected.update(baseline["windows_only_tests"])
    else:
        expected.update(baseline["unix_only_tests"])
    if include_solari:
        expected.update(baseline["solari_feature_tests"])
    return expected


def parse_cargo_list_output(raw_text: str, *, single_target: bool = False) -> tuple[Counter[str], Counter[str]]:
    """Validate every complete harness section, not merely presence of a footer."""
    tests: Counter[str] = Counter()
    benchmarks: Counter[str] = Counter()
    section_tests = section_benches = sections = 0
    pending = False
    for line in raw_text.splitlines():
        line = line.strip()
        if not line:
            continue
        if single_target and sections:
            raise BaselineError("record after terminal target summary")
        match = SUMMARY_LINE_RE.fullmatch(line)
        if match:
            if (int(match[1]), int(match[2])) != (section_tests, section_benches):
                raise BaselineError("listing summary count mismatch")
            sections += 1
            section_tests = section_benches = 0
            pending = False
        elif line.endswith(TEST_SUFFIX) and line[:-len(TEST_SUFFIX)].strip():
            tests[line[:-len(TEST_SUFFIX)]] += 1
            section_tests += 1
            pending = True
        elif line.endswith(BENCHMARK_SUFFIX) and line[:-len(BENCHMARK_SUFFIX)].strip():
            benchmarks[line[:-len(BENCHMARK_SUFFIX)]] += 1
            section_benches += 1
            pending = True
        else:
            raise BaselineError(f"unsupported listing record: {line!r}")
    if not sections or pending:
        raise BaselineError("missing terminal cargo test summary line")
    return tests, benchmarks


def compare_enumerations(
    expected: Counter[str],
    observed_tests: Counter[str],
    observed_benchmarks: Counter[str],
) -> dict[str, Any]:
    missing: list[dict[str, Any]] = []
    unexpected: list[dict[str, Any]] = []

    for name in sorted(set(expected) | set(observed_tests)):
        exp_count = expected.get(name, 0)
        obs_count = observed_tests.get(name, 0)
        if obs_count < exp_count:
            missing.append(
                {
                    "name": name,
                    "expected_count": exp_count,
                    "observed_count": obs_count,
                }
            )
        elif obs_count > exp_count:
            unexpected.append(
                {
                    "name": name,
                    "expected_count": exp_count,
                    "observed_count": obs_count,
                }
            )

    unexpected_benchmarks = [
        {"name": name, "observed_count": count}
        for name, count in sorted(observed_benchmarks.items())
    ]

    violation_count = len(missing) + len(unexpected) + len(unexpected_benchmarks)
    return {
        "status": "pass" if violation_count == 0 else "fail",
        "expected_total_entries": sum(expected.values()),
        "expected_unique_identities": len(expected),
        "observed_total_entries": sum(observed_tests.values()),
        "observed_unique_identities": len(observed_tests),
        "observed_benchmark_entries": sum(observed_benchmarks.values()),
        "missing": missing,
        "unexpected": unexpected,
        "unexpected_benchmarks": unexpected_benchmarks,
        "violation_count": violation_count,
    }


def run_cargo_test_list(root: Path, *, include_solari: bool = False) -> str:
    cmd = ["cargo", "test", "--workspace", "--all-targets", "--locked"]
    if include_solari:
        cmd.extend(["--features", "solari"])
    cmd.extend(["--", "--list"])

    proc = subprocess.run(
        cmd,
        cwd=str(root),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="strict",
        check=False,
    )
    if proc.returncode != 0:
        raise BaselineError(
            f"command {' '.join(cmd)} failed with exit code {proc.returncode}:\n{proc.stderr}"
        )
    return proc.stdout


def evaluate_legacy_parity(
    *,
    root: Path,
    baseline_path: Path,
    platform_arg: str = "auto",
    list_file: Path | None = None,
    include_solari: bool = False,
) -> dict[str, Any]:
    # Compatibility diagnostic for the schema-1 unit tests only. Not a CLI gate.
    platform = detect_platform(platform_arg)
    baseline = load_baseline(baseline_path)
    expected = expected_counts_for_platform(
        baseline,
        platform,
        include_solari=include_solari,
    )
    if list_file is not None:
        raw_output = list_file.read_text(encoding="utf-8")
        source = str(list_file)
    else:
        raw_output = run_cargo_test_list(root, include_solari=include_solari)
        source = "cargo test --workspace --all-targets --locked" + (
            " --features solari" if include_solari else ""
        ) + " -- --list"

    observed_tests, observed_benchmarks = parse_cargo_list_output(raw_output)
    comparison = compare_enumerations(expected, observed_tests, observed_benchmarks)
    return {
        "baseline_checkpoint": baseline["baseline_checkpoint"],
        "baseline_commit": baseline["baseline_commit"],
        "platform": platform,
        "include_solari": include_solari,
        "source": source,
        **comparison,
    }


# Schema 2: one independently listed standard test harness per stable target.
BASELINE_COMMIT = "59ccddbea7be85a34a8e8f1d4d131c91c7ade858"
BASELINE_CHECKPOINT = "BO-M9-C1"
PINS = {"bevy_frost": "da3110c07ba8d79d6deeba5a68dd536231956fb9",
        "bevy_glacial": "6bef365be227e1461c1f8019e3b8c631c166fd44"}
CONFIGS = {f"{os_name}:{feature}" for os_name in SUPPORTED_PLATFORMS
           for feature in ("default", "solari")}
IDENTITY_MIGRATIONS_FILE = "test_enumeration_identity_migrations.json"
MIGRATION_CHECKPOINTS = frozenset(
    {
        "BOR-C2",
        "BOR-C3",
        "BOR-C4",
        "BOR-C5",
        "BOR-C7",
        "BOV-C3",
        "BOV-C4",
        "BOV-C5",
        "BOV-C6",
        "BOV-C7",
        "BOV-C8",
    }
)


def checked_run(cmd, cwd, *, env=None):
    try:
        result = subprocess.run(cmd, cwd=str(cwd), capture_output=True, text=True,
                                encoding="utf-8", errors="strict", check=False, env=env)
    except (OSError, UnicodeError) as exc:
        raise BaselineError(f"cannot run {cmd!r}: {exc}") from exc
    if result.returncode:
        raise BaselineError(f"{cmd!r} failed with exit code {result.returncode}: {result.stderr}")
    return result.stdout


def read_json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise BaselineError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique)
    except (OSError, UnicodeError, ValueError) as exc:
        raise BaselineError(f"invalid JSON {path}: {exc}") from exc


def config_key(platform, solari):
    return f"{platform}:{'solari' if solari else 'default'}"


def stable_target(package_path, target):
    kinds = target.get("kind")
    name = target.get("name")
    src = target.get("src_path")
    if not isinstance(kinds, list) or not kinds or not all(isinstance(k, str) and k for k in kinds):
        raise BaselineError("invalid Cargo target kind")
    if not isinstance(name, str) or not name or not isinstance(src, str):
        raise BaselineError("invalid Cargo target name/source")
    try:
        Path(src).resolve().relative_to(package_path.parent)
    except ValueError as exc:
        raise BaselineError("target source is outside its package") from exc
    # Source path is checked for containment, not part of identity: moves within
    # the same Cargo target do not change the test harness identity.
    return (tuple(sorted(kinds)), name)


def discover_executables(root, solari):
    """Build the complete Cargo-selected test graph, bind JSON artifacts to metadata.

    Cargo's test-profile executable artifacts, not log arrival order, identify
    harnesses. Unsupported/custom harnesses fail at listing validation.
    """
    root = root.resolve()
    features = ["--features", "solari"] if solari else []
    meta = json.loads(checked_run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked", *features], root))
    members = set(meta["workspace_members"])
    packages = {p["id"]: p for p in meta["packages"] if p["id"] in members}
    if set(packages) != members or not members:
        raise BaselineError("incomplete workspace metadata")
    known = {}
    for pid, package in packages.items():
        manifest = Path(package["manifest_path"]).resolve()
        try:
            relative = manifest.relative_to(root).as_posix()
        except ValueError as exc:
            raise BaselineError("workspace manifest outside root") from exc
        for target in package["targets"]:
            kinds, name = stable_target(manifest, target)
            key = (relative, kinds, name)
            index = (pid, kinds, name)
            if index in known:
                raise BaselineError(f"ambiguous metadata target {index}")
            known[index] = (key, manifest.parent)
    cmd = ["cargo", "test", "--workspace", "--all-targets", "--locked", *features,
           "--no-run", "--message-format=json"]
    output = checked_run(cmd, root)
    found = {}
    paths = set()
    finished = False
    for line in output.splitlines():
        if not line.strip():
            continue
        msg = json.loads(line)
        if not isinstance(msg, dict):
            raise BaselineError("non-object Cargo message")
        if finished:
            raise BaselineError("unexpected Cargo message after build-finished")
        if msg.get("reason") == "build-finished":
            if finished or msg.get("success") is not True:
                raise BaselineError("invalid Cargo build-finished receipt")
            finished = True
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
            continue
        pid = msg.get("package_id")
        if pid not in packages or not msg.get("profile", {}).get("test"):
            continue
        kinds, name = stable_target(Path(packages[pid]["manifest_path"]).resolve(), msg["target"])
        index = (pid, kinds, name)
        if index not in known:
            raise BaselineError(f"unrecognized Cargo artifact: {index}")
        key, cwd = known[index]
        exe = Path(msg["executable"]).resolve()
        if key in found or exe in paths or not exe.is_file():
            raise BaselineError(f"duplicate/missing executable for {key}")
        found[key] = (exe, cwd)
        paths.add(exe)
    if not finished or not found:
        raise BaselineError("no complete Cargo test executable inventory")
    return found


@functools.lru_cache(maxsize=1)
def _rustc_target_libdir() -> str | None:
    try:
        proc = subprocess.run(
            ["rustc", "--print", "target-libdir"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
    except OSError:
        return None
    if proc.returncode != 0:
        return None
    libdir = proc.stdout.strip()
    if not libdir:
        return None
    resolved = Path(libdir).resolve()
    return str(resolved) if resolved.is_dir() else None


def harness_env(executable):
    # Cargo supplies loader search paths for test executables, including
    # rustc's target-libdir for dynamically linked proc-macro test harnesses.
    env = os.environ.copy()
    dirs: list[str] = []
    for candidate in (
        str(executable.parent),
        str(executable.parent.parent),
        _rustc_target_libdir(),
    ):
        if candidate and candidate not in dirs:
            dirs.append(candidate)
    for var in ("PATH", "LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"):
        existing = env.get(var)
        parts = [p for p in existing.split(os.pathsep) if p] if existing else []
        merged = dirs + [p for p in parts if p not in dirs]
        env[var] = os.pathsep.join(merged)
    return env


def capture(root, solari):
    executables = discover_executables(root, solari)
    targets = []
    for key, (exe, cwd) in sorted(executables.items()):
        listing = checked_run([str(exe), "--list", "--color", "never"], cwd,
                              env=harness_env(exe))
        parse_cargo_list_output(listing, single_target=True)
        targets.append({"manifest": key[0], "kind": list(key[1]), "target": key[2],
                        "listing": listing})
    # Cross-check the independently bound artifact inventory against Cargo's
    # complete ordinary listing, including any custom harnesses not recognized
    # as test-profile artifacts. This cannot replace target-qualified parity.
    raw = run_cargo_test_list(root, include_solari=solari)
    aggregate, benchmarks = parse_cargo_list_output(raw)
    _, qualified, qualified_bench = parse_targets(targets)
    footer_count = sum(bool(SUMMARY_LINE_RE.fullmatch(line.strip())) for line in raw.splitlines())
    if (footer_count != len(targets) or aggregate != flatten(qualified)
            or benchmarks != flatten(qualified_bench)):
        raise BaselineError("Cargo aggregate listing and target artifact capture disagree")
    return targets, executables


def parse_targets(raw):
    if not isinstance(raw, list) or not raw:
        raise BaselineError("missing target-qualified capture")
    targets = set()
    tests, benches = Counter(), Counter()
    for entry in raw:
        if not isinstance(entry, dict) or set(entry) != {"manifest", "kind", "target", "listing"}:
            raise BaselineError("invalid target listing envelope")
        manifest, kinds, name, listing = (entry[k] for k in ("manifest", "kind", "target", "listing"))
        if (not isinstance(manifest, str) or manifest.split("/")[-1] != "Cargo.toml"
                or "\\" in manifest or ":" in manifest or manifest.startswith("/")
                or any(part in ("", ".", "..") for part in manifest.split("/"))
                or not isinstance(kinds, list) or not kinds
                or not all(isinstance(k, str) and k for k in kinds)
                or kinds != sorted(set(kinds)) or not isinstance(name, str) or not name
                or not isinstance(listing, str)):
            raise BaselineError("invalid stable target identity/listing")
        key = (manifest, tuple(kinds), name)
        if key in targets:
            raise BaselineError(f"duplicate target {key}")
        targets.add(key)
        t, b = parse_cargo_list_output(listing, single_target=True)
        for ident, n in t.items():
            tests[(*key, ident)] = n
        for ident, n in b.items():
            benches[(*key, ident)] = n
    return targets, tests, benches


def flatten(tests):
    counts = Counter()
    for key, count in tests.items():
        counts[key[-1]] += count
    return counts


def _migration_target_key(raw):
    if not isinstance(raw, dict) or set(raw) != {"manifest", "kind", "target"}:
        raise BaselineError("invalid identity migration target")
    manifest, kinds, target = (raw[key] for key in ("manifest", "kind", "target"))
    if (
        not isinstance(manifest, str)
        or manifest.split("/")[-1] != "Cargo.toml"
        or "\\" in manifest
        or ":" in manifest
        or manifest.startswith("/")
        or any(part in ("", ".", "..") for part in manifest.split("/"))
        or not isinstance(kinds, list)
        or not kinds
        or not all(isinstance(kind, str) and kind for kind in kinds)
        or kinds != sorted(set(kinds))
        or not isinstance(target, str)
        or not target
    ):
        raise BaselineError("invalid identity migration target")
    return (manifest, tuple(kinds), target)


def load_identity_migrations(path, configs):
    """Load exact aliases whose source identities are present in the pinned baseline.

    The configuration list on each alias is checked against the actual six
    qualified captures. This keeps platform/feature applicability explicit and
    prevents a stale map from silently being applied to a different inventory.
    """
    data = read_json(path)
    if (
        not isinstance(data, dict)
        or data.get("schema_version") != 1
        or data.get("baseline_checkpoint") != BASELINE_CHECKPOINT
        or data.get("baseline_commit") != BASELINE_COMMIT
        or not isinstance(data.get("entries"), list)
    ):
        raise BaselineError("identity migration schema/provenance mismatch")
    if not isinstance(configs, dict) or set(configs) != CONFIGS:
        raise BaselineError("identity migrations require all six qualified baseline configurations")

    baseline_tests = {}
    for config in CONFIGS:
        _, tests, benchmarks = parse_targets(configs[config]["targets"])
        if benchmarks:
            raise BaselineError(f"baseline contains benchmarks for {config}")
        baseline_tests[config] = tests

    migrations = []
    source_keys = set()
    destination_keys = set()
    for index, raw in enumerate(data["entries"]):
        required = {"old_name", "new_name", "targets", "configurations", "evidence"}
        if not isinstance(raw, dict) or set(raw) != required:
            raise BaselineError(f"identity migration entries[{index}] has invalid fields")
        old_name, new_name = raw["old_name"], raw["new_name"]
        if (
            not isinstance(old_name, str)
            or not old_name.strip()
            or old_name.endswith(TEST_SUFFIX)
            or not isinstance(new_name, str)
            or not new_name.strip()
            or new_name.endswith(TEST_SUFFIX)
            or old_name == new_name
        ):
            raise BaselineError(f"identity migration entries[{index}] has invalid names")

        raw_targets = raw["targets"]
        if not isinstance(raw_targets, list) or not raw_targets:
            raise BaselineError(f"identity migration entries[{index}].targets must be non-empty")
        targets = tuple(_migration_target_key(target) for target in raw_targets)
        if len(set(targets)) != len(targets) or list(targets) != sorted(set(targets)):
            raise BaselineError(f"identity migration entries[{index}].targets must be unique and sorted")

        raw_configs = raw["configurations"]
        if (
            not isinstance(raw_configs, list)
            or not raw_configs
            or not all(isinstance(config, str) and config in CONFIGS for config in raw_configs)
            or raw_configs != sorted(set(raw_configs))
        ):
            raise BaselineError(f"identity migration entries[{index}].configurations are invalid")
        configurations = frozenset(raw_configs)

        evidence = raw["evidence"]
        if (
            not isinstance(evidence, list)
            or not evidence
            or not all(isinstance(item, str) and item in MIGRATION_CHECKPOINTS for item in evidence)
            or len(evidence) != len(set(evidence))
        ):
            raise BaselineError(f"identity migration entries[{index}].evidence is invalid")

        entry_sources = {(manifest, kinds, target, old_name) for manifest, kinds, target in targets}
        entry_destinations = {(manifest, kinds, target, new_name) for manifest, kinds, target in targets}
        if source_keys & entry_sources or destination_keys & entry_destinations:
            raise BaselineError("identity migration source or destination is duplicated")
        if source_keys & entry_destinations or destination_keys & entry_sources:
            raise BaselineError("identity migration chains and collisions are not supported")

        actual_configurations = set()
        for config, tests in baseline_tests.items():
            present_counts = [tests.get(key, 0) for key in entry_sources]
            any_present = any(count > 0 for count in present_counts)
            all_present = all(count > 0 for count in present_counts)
            if any_present and not all_present:
                raise BaselineError(
                    f"identity migration {old_name!r} is present in only some listed targets for {config}"
                )
            if all_present:
                actual_configurations.add(config)
                for key in entry_destinations:
                    if tests.get(key, 0):
                        raise BaselineError(
                            f"identity migration {new_name!r} collides with a pinned baseline identity"
                        )

        if actual_configurations != configurations:
            raise BaselineError(
                f"identity migration {old_name!r} configuration scope disagrees with baseline: "
                f"declared={sorted(configurations)}, actual={sorted(actual_configurations)}"
            )
        if not actual_configurations:
            raise BaselineError(f"identity migration source {old_name!r} is absent from the baseline")

        source_keys.update(entry_sources)
        destination_keys.update(entry_destinations)
        migrations.append(
            {
                "old_name": old_name,
                "new_name": new_name,
                "targets": targets,
                "configurations": configurations,
                "evidence": tuple(evidence),
            }
        )

    if source_keys & destination_keys:
        raise BaselineError("identity migration chains are not supported")
    return migrations


def apply_identity_migrations(expected_tests, migrations, configuration):
    if configuration not in CONFIGS:
        raise BaselineError(f"unsupported identity migration configuration {configuration!r}")
    transformed = Counter(expected_tests)
    applied = 0
    for migration in migrations:
        old_name = migration["old_name"]
        new_name = migration["new_name"]
        applies = configuration in migration["configurations"]
        source_keys = [(*target, old_name) for target in migration["targets"]]
        destination_keys = [(*target, new_name) for target in migration["targets"]]
        present = [transformed.get(key, 0) > 0 for key in source_keys]
        if any(present) != applies or (applies and not all(present)):
            raise BaselineError(
                f"identity migration {old_name!r} does not match expected source presence in {configuration}"
            )
        if not applies:
            continue
        if any(transformed.get(key, 0) for key in destination_keys):
            raise BaselineError(f"identity migration {new_name!r} collides in {configuration}")
        for source, destination in zip(source_keys, destination_keys, strict=True):
            transformed[destination] = transformed.pop(source)
            applied += 1
    return transformed, applied


def qualified_report(expected, observed, *, identity_migrations=None, configuration=None):
    et, en, eb = parse_targets(expected)
    ot, on, ob = parse_targets(observed)
    if eb:
        raise BaselineError("baseline contains unexpected benchmarks")
    migrations_applied = None
    if identity_migrations is not None:
        if configuration is None:
            raise BaselineError("identity migrations require an explicit configuration")
        en, migrations_applied = apply_identity_migrations(en, identity_migrations, configuration)
    report = compare_enumerations(en, on, ob)
    # Existing report keys preserved; missing/unexpected names now structured.
    for field in ("missing", "unexpected", "unexpected_benchmarks"):
        for item in report[field]:
            manifest, kinds, target, name = item.pop("name")
            item.update(manifest=manifest, kind=list(kinds), target=target, name=name)
    report["missing_targets"] = sorted(et - ot)
    report["unexpected_targets"] = sorted(ot - et)
    report["violation_count"] += len(et - ot) + len(ot - et)
    report["status"] = "fail" if report["violation_count"] else "pass"
    report["expected_unique_names"] = len(flatten(en))
    report["observed_unique_names"] = len(flatten(on))
    if migrations_applied is not None:
        report["identity_migrations_applied"] = migrations_applied
    return report


def assert_baseline_checkout(root):
    for path, sha in [(root, BASELINE_COMMIT), *[(root.parent / n, sha) for n, sha in PINS.items()]]:
        if checked_run(["git", "rev-parse", "HEAD"], path).strip() != sha:
            raise BaselineError(f"baseline capture requires exact clean commit {sha} at {path}")
        if checked_run(["git", "status", "--porcelain", "--untracked-files=all"], path).strip():
            raise BaselineError(f"baseline capture requires clean checkout at {path}")


def capture_document(root, solari):
    assert_baseline_checkout(root)
    targets, _ = capture(root, solari)
    assert_baseline_checkout(root)
    return {"schema_version": 2, "baseline_checkpoint": BASELINE_CHECKPOINT,
            "baseline_commit": BASELINE_COMMIT, "siblings": PINS,
            "configuration": config_key(detect_platform(), solari), "targets": targets}


def validate_capture(doc, config):
    if (not isinstance(doc, dict) or doc.get("schema_version") != 2
            or doc.get("baseline_checkpoint") != BASELINE_CHECKPOINT
            or doc.get("baseline_commit") != BASELINE_COMMIT or doc.get("siblings") != PINS
            or doc.get("configuration") != config):
        raise BaselineError("capture identity/configuration mismatch")
    parse_targets(doc.get("targets"))
    return doc["targets"]


def check_legacy_projection(targets, legacy, config):
    platform, feature = config.split(":")
    _, tests, benches = parse_targets(targets)
    if benches or flatten(tests) != expected_counts_for_platform(legacy, platform, include_solari=feature == "solari"):
        raise BaselineError(f"qualified capture disagrees with original aggregate baseline for {config}; Owner disposition required")


def load_qualified(path, legacy):
    doc = read_json(path)
    if (not isinstance(doc, dict) or doc.get("schema_version") != 2
            or doc.get("baseline_commit") != BASELINE_COMMIT
            or doc.get("baseline_checkpoint") != BASELINE_CHECKPOINT
            or doc.get("siblings") != PINS or not isinstance(doc.get("configurations"), dict)
            or set(doc["configurations"]) != CONFIGS):
        raise BaselineError("qualified baseline schema/provenance incomplete; capture all six configurations")
    configs = doc["configurations"]
    for config, envelope in configs.items():
        check_legacy_projection(validate_capture(envelope, config), legacy, config)
    return configs


def merge_captures(paths, output, legacy):
    configs = {}
    for path in paths:
        doc = read_json(path)
        config = doc.get("configuration") if isinstance(doc, dict) else None
        if config not in CONFIGS or config in configs:
            raise BaselineError("duplicate/unknown baseline capture configuration")
        check_legacy_projection(validate_capture(doc, config), legacy, config)
        configs[config] = doc
    if set(configs) != CONFIGS:
        raise BaselineError(f"missing baseline configurations: {sorted(CONFIGS-set(configs))}")
    doc = {"schema_version": 2, "baseline_checkpoint": BASELINE_CHECKPOINT,
           "baseline_commit": BASELINE_COMMIT, "siblings": PINS, "configurations": configs}
    # No overwrite or partial update of the baseline.
    with output.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(doc, handle, indent=2, sort_keys=True)
        handle.write("\n")


def windows_required(configs, legacy, *, identity_migrations=None):
    _, tests, _ = parse_targets(configs["windows:default"]["targets"])
    required_names = Counter(legacy["windows_only_tests"])
    if identity_migrations is not None:
        tests, _ = apply_identity_migrations(tests, identity_migrations, "windows:default")
        for migration in identity_migrations:
            old_name = migration["old_name"]
            new_name = migration["new_name"]
            if "windows:default" not in migration["configurations"] or old_name not in required_names:
                continue
            count = required_names.pop(old_name)
            if new_name in required_names:
                raise BaselineError(f"Windows-only identity migration collides for {new_name!r}")
            required_names[new_name] = count
    required = Counter({key: n for key, n in tests.items() if key[-1] in required_names})
    if not required or flatten(required) != required_names or any(n != 1 for n in required.values()):
        raise BaselineError("Windows required target/name inventory is incomplete or ambiguous")
    return required


def execute_windows(configs, legacy, executables, *, identity_migrations=None):
    required = windows_required(configs, legacy, identity_migrations=identity_migrations)
    receipts = []
    for key in sorted(required):
        if key[:3] not in executables:
            raise BaselineError(f"missing Windows test harness {key[:3]}")
        exe, cwd = executables[key[:3]]
        name = key[-1]
        output = checked_run([str(exe), "--exact", name, "--include-ignored", "--color", "never"], cwd,
                             env=harness_env(exe))
        if (not re.search(r"^running 1 test$", output, re.M)
                or not re.search(r"^test " + re.escape(name) + r" \.\.\. ok$", output, re.M)
                or not re.search(r"^test result: ok\. 1 passed; 0 failed; 0 ignored;", output, re.M)):
            raise BaselineError(f"Windows test was not demonstrably executed: {key}")
        receipts.append(key)
    return receipts


def main(argv=None):
    parser = argparse.ArgumentParser(description="Target-qualified test enumeration parity")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--baseline", type=Path, help="schema 2 qualified baseline")
    parser.add_argument("--legacy-baseline", type=Path, help="original schema 1 aggregate cross-check")
    parser.add_argument("--migrations", type=Path, help="exact identity migration map")
    parser.add_argument("--list-file", type=Path, help="schema 2 capture envelope, not flat stdout")
    parser.add_argument("--platform", choices=("auto", "windows", "linux", "macos"), default="auto")
    parser.add_argument("--include-solari", action="store_true")
    parser.add_argument("--json", action="store_true")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--capture-baseline", type=Path, metavar="OUTPUT")
    mode.add_argument("--merge-baseline", nargs="+", type=Path, metavar="CAPTURE")
    mode.add_argument("--execute-windows", action="store_true")
    args = parser.parse_args(argv)
    root = args.root.resolve()
    baseline = args.baseline or root / "scripts/test_enumeration_qualified_baseline.json"
    legacy_path = args.legacy_baseline or root / "scripts/test_enumeration_baseline.json"
    try:
        platform = detect_platform(args.platform)
        config = config_key(platform, args.include_solari)
        if (not args.list_file or args.capture_baseline or args.execute_windows) and platform != detect_platform():
            raise BaselineError("live enumeration must use actual host OS")
        if args.capture_baseline:
            if args.list_file:
                raise BaselineError("baseline capture cannot use offline listing")
            doc = capture_document(root, args.include_solari)
            with args.capture_baseline.open("x", encoding="utf-8", newline="\n") as handle:
                json.dump(doc, handle, indent=2, sort_keys=True)
                handle.write("\n")
            return 0
        legacy = load_baseline(legacy_path)
        if legacy["baseline_commit"] != BASELINE_COMMIT or legacy["baseline_checkpoint"] != BASELINE_CHECKPOINT:
            raise BaselineError("original baseline identity mismatch")
        if args.merge_baseline:
            if args.list_file:
                raise BaselineError("merge cannot use offline list option")
            merge_captures(args.merge_baseline, baseline, legacy)
            return 0
        configs = load_qualified(baseline, legacy)
        if args.execute_windows and (config != "windows:default" or args.list_file):
            raise BaselineError("Windows execution requires live default Windows capture")
        migrations_path = args.migrations or root / "scripts" / IDENTITY_MIGRATIONS_FILE
        migrations = load_identity_migrations(migrations_path, configs)
        if args.list_file:
            observed = validate_capture(read_json(args.list_file), config)
            executables = {}
        else:
            observed, executables = capture(root, args.include_solari)
        report = qualified_report(
            configs[config]["targets"],
            observed,
            identity_migrations=migrations,
            configuration=config,
        )
        report.update(baseline_checkpoint=BASELINE_CHECKPOINT, baseline_commit=BASELINE_COMMIT,
                      platform=platform, include_solari=args.include_solari,
                      source="offline qualified capture" if args.list_file else "live Cargo test artifacts",
                      live_capture=not bool(args.list_file))
        if args.execute_windows and report["status"] == "pass":
            report["executed_windows_tests"] = execute_windows(
                configs,
                legacy,
                executables,
                identity_migrations=migrations,
            )
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if report["status"] == "pass" else 1
    except (BaselineError, OSError, UnicodeError, ValueError, KeyError, TypeError) as exc:
        if args.json:
            print(json.dumps({"status": "error", "error": str(exc)}, indent=2))
        else:
            print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
