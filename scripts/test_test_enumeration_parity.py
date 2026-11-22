#!/usr/bin/env python3
"""Unit and adversarial tests for scripts/check_test_enumeration_parity.py."""

from __future__ import annotations

from collections import Counter
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_test_enumeration_parity as parity


REPO_ROOT = Path(__file__).resolve().parents[1]
CHECKED_IN_BASELINE = REPO_ROOT / "scripts" / "test_enumeration_baseline.json"


def _make_baseline(
    *,
    common: dict[str, int] | None = None,
    windows_only: dict[str, int] | None = None,
    unix_only: dict[str, int] | None = None,
    solari_feature: dict[str, int] | None = None,
    summary_overrides: dict[str, int] | None = None,
) -> dict:
    common_map = {"alpha::unit": 2, "beta::contract": 1} if common is None else common
    win_map = {"win_only::path_case": 2} if windows_only is None else windows_only
    unix_map = {"unix_only::symlink_case": 2} if unix_only is None else unix_only
    solari_map = {"solari::scene_test": 2} if solari_feature is None else solari_feature

    def _bucket(mapping: dict[str, int]) -> list[dict]:
        return [{"name": name, "count": count} for name, count in sorted(mapping.items())]

    summary = {
        "common_total_entries": sum(common_map.values()),
        "common_unique_identities": len(common_map),
        "windows_only_total_entries": sum(win_map.values()),
        "windows_only_unique_identities": len(win_map),
        "unix_only_total_entries": sum(unix_map.values()),
        "unix_only_unique_identities": len(unix_map),
        "solari_feature_total_entries": sum(solari_map.values()),
        "solari_feature_unique_identities": len(solari_map),
        "windows_default_total_entries": sum(common_map.values()) + sum(win_map.values()),
        "windows_default_unique_identities": len(common_map) + len(win_map),
        "unix_default_total_entries": sum(common_map.values()) + sum(unix_map.values()),
        "unix_default_unique_identities": len(common_map) + len(unix_map),
    }
    if summary_overrides:
        summary.update(summary_overrides)

    return {
        "schema_version": 1,
        "baseline_checkpoint": "BO-M9-C1",
        "baseline_commit": "59ccddbea7be85a34a8e8f1d4d131c91c7ade858",
        "command": "cargo test --workspace --all-targets --locked -- --list",
        "summary": summary,
        "common_tests": _bucket(common_map),
        "windows_only_tests": _bucket(win_map),
        "unix_only_tests": _bucket(unix_map),
        "solari_feature_tests": _bucket(solari_map),
    }


def _render_list(counts: Counter[str], benchmarks: list[str] | None = None) -> str:
    lines: list[str] = []
    for name in sorted(counts):
        for _ in range(counts[name]):
            lines.append(f"{name}: test")
    bench_list = benchmarks or []
    for bench_name in bench_list:
        lines.append(f"{bench_name}: benchmark")
    lines.append(f"{sum(counts.values())} tests, {len(bench_list)} benchmarks")
    return "\n".join(lines) + "\n"


def _target(name, counts):
    return {"manifest":"Cargo.toml", "kind":["lib"], "target":name, "listing":_render_list(Counter(counts))}


def _envelope(config, targets):
    return {"schema_version":2, "baseline_checkpoint":parity.BASELINE_CHECKPOINT,
            "baseline_commit":parity.BASELINE_COMMIT, "siblings":parity.PINS,
            "configuration":config, "targets":targets}


def _baseline_doc(configs):
    return {"schema_version":2, "baseline_checkpoint":parity.BASELINE_CHECKPOINT,
            "baseline_commit":parity.BASELINE_COMMIT, "siblings":parity.PINS, "configurations":configs}


def _qualified_fixture(root):
    legacy_path = root / "legacy.json"
    legacy_path.write_text(json.dumps(_make_baseline()), encoding="utf-8")
    legacy = parity.load_baseline(legacy_path)
    configs = {}
    for config in parity.CONFIGS:
        platform, feature = config.split(":")
        counts = parity.expected_counts_for_platform(legacy, platform, include_solari=feature=="solari")
        configs[config] = _envelope(config, [_target("app", counts)])
    baseline = root / "qualified.json"
    baseline.write_text(json.dumps(_baseline_doc(configs)), encoding="utf-8")
    scripts = root / "scripts"
    scripts.mkdir(exist_ok=True)
    (scripts / parity.IDENTITY_MIGRATIONS_FILE).write_text(
        json.dumps(
            {
                "schema_version": 1,
                "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                "baseline_commit": parity.BASELINE_COMMIT,
                "entries": [],
            }
        ),
        encoding="utf-8",
    )
    return legacy_path, baseline, configs


class TestEnumerationParityTests(unittest.TestCase):
    def test_checked_in_baseline_manifest_invariants(self) -> None:
        raw_bytes = CHECKED_IN_BASELINE.read_bytes()
        self.assertNotIn(b"\r\n", raw_bytes, "baseline manifest must use LF line endings")

        baseline = parity.load_baseline(CHECKED_IN_BASELINE)
        self.assertEqual(baseline["baseline_checkpoint"], "BO-M9-C1")
        self.assertEqual(
            baseline["baseline_commit"], "59ccddbea7be85a34a8e8f1d4d131c91c7ade858"
        )
        summary = baseline["summary"]
        self.assertEqual(summary["common_total_entries"], 1958)
        self.assertEqual(summary["common_unique_identities"], 1181)
        self.assertEqual(summary["windows_only_total_entries"], 8)
        self.assertEqual(summary["windows_only_unique_identities"], 4)
        self.assertEqual(summary["unix_only_total_entries"], 22)
        self.assertEqual(summary["unix_only_unique_identities"], 11)
        self.assertEqual(summary["solari_feature_total_entries"], 16)
        self.assertEqual(summary["solari_feature_unique_identities"], 8)
        self.assertEqual(summary["windows_default_total_entries"], 1966)
        self.assertEqual(summary["windows_default_unique_identities"], 1185)
        self.assertEqual(summary["unix_default_total_entries"], 1980)
        self.assertEqual(summary["unix_default_unique_identities"], 1192)

    def test_checked_in_baseline_synthetic_roundtrip_all_platforms(self) -> None:
        baseline = parity.load_baseline(CHECKED_IN_BASELINE)
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            for platform, include_solari, expected_total, expected_unique in (
                ("windows", False, 1966, 1185),
                ("linux", False, 1980, 1192),
                ("macos", False, 1980, 1192),
                ("linux", True, 1996, 1200),
            ):
                expected = parity.expected_counts_for_platform(
                    baseline, platform, include_solari=include_solari
                )
                list_path = tmp_dir / f"{platform}_{include_solari}.txt"
                list_path.write_text(_render_list(expected), encoding="utf-8")
                report = parity.evaluate_legacy_parity(
                    root=REPO_ROOT,
                    baseline_path=CHECKED_IN_BASELINE,
                    platform_arg=platform,
                    list_file=list_path,
                    include_solari=include_solari,
                )
                self.assertEqual(report["status"], "pass")
                self.assertEqual(report["violation_count"], 0)
                self.assertEqual(report["expected_total_entries"], expected_total)
                self.assertEqual(report["expected_unique_identities"], expected_unique)

    def test_cli_passes_on_exact_match_and_fails_on_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, baseline_path, configs = _qualified_fixture(root)
            envelope = configs["linux:default"]
            good = root / "good.json"
            good.write_text(json.dumps(envelope), encoding="utf-8")
            args = ["--root", str(root), "--legacy-baseline", str(legacy_path), "--baseline", str(baseline_path), "--platform", "linux", "--json", "--list-file"]
            with contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(parity.main(args + [str(good)]), 0)
            self.assertEqual(json.loads(out.getvalue())["violation_count"], 0)
            bad = root / "bad.json"
            changed = json.loads(json.dumps(envelope))
            counts, _ = parity.parse_cargo_list_output(changed["targets"][0]["listing"])
            counts["alpha::unit"] -= 1
            changed["targets"][0]["listing"] = _render_list(counts)
            bad.write_text(json.dumps(changed), encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(parity.main(args + [str(bad)]), 1)
            report = json.loads(out.getvalue())
            self.assertEqual(report["missing"][0]["name"], "alpha::unit")
            self.assertEqual(report["missing"][0]["target"], "app")
            self.assertEqual(report["missing"][0]["observed_count"], 1)

    def test_cli_applies_the_pinned_target_qualified_migration_map(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, baseline_path, _ = _qualified_fixture(root)
            migration_path = root / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [
                            {
                                "old_name": "alpha::unit",
                                "new_name": "alpha::relocated",
                                "targets": [
                                    {"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"}
                                ],
                                "configurations": sorted(parity.CONFIGS),
                                "evidence": ["BOV-C4"],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            observed = root / "observed.json"
            envelope = _envelope(
                "linux:default",
                [_target("app", {"alpha::relocated": 2, "beta::contract": 1,
                                 "unix_only::symlink_case": 2})],
            )
            observed.write_text(json.dumps(envelope), encoding="utf-8")
            args = [
                "--root", str(root),
                "--legacy-baseline", str(legacy_path),
                "--baseline", str(baseline_path),
                "--platform", "linux",
                "--list-file", str(observed),
                "--json",
            ]
            with contextlib.redirect_stdout(io.StringIO()) as out:
                code = parity.main(args)
            self.assertEqual(code, 0)
            report = json.loads(out.getvalue())
            self.assertEqual(report["status"], "pass")
            self.assertEqual(report["identity_migrations_applied"], 1)

    def test_detects_missing_renamed_extra_and_benchmark_entries(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            baseline_path = tmp_dir / "baseline.json"
            baseline_path.write_text(json.dumps(_make_baseline()), encoding="utf-8")
            baseline = parity.load_baseline(baseline_path)
            expected = parity.expected_counts_for_platform(baseline, "windows")

            mutated = Counter(expected)
            del mutated["beta::contract"]
            mutated["beta::renamed_contract"] = 1
            mutated["extra::sneaky_test"] = 1
            list_path = tmp_dir / "mutated.txt"
            list_path.write_text(
                _render_list(mutated, benchmarks=["unexpected::bench"]), encoding="utf-8"
            )

            report = parity.evaluate_legacy_parity(
                root=tmp_dir,
                baseline_path=baseline_path,
                platform_arg="windows",
                list_file=list_path,
            )
            self.assertEqual(report["status"], "fail")
            self.assertEqual(report["violation_count"], 4)
            self.assertEqual(
                report["missing"],
                [{"name": "beta::contract", "expected_count": 1, "observed_count": 0}],
            )
            self.assertEqual(
                report["unexpected"],
                [
                    {"name": "beta::renamed_contract", "expected_count": 0, "observed_count": 1},
                    {"name": "extra::sneaky_test", "expected_count": 0, "observed_count": 1},
                ],
            )
            self.assertEqual(
                report["unexpected_benchmarks"],
                [{"name": "unexpected::bench", "observed_count": 1}],
            )

    def test_platform_and_solari_mismatch_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            baseline_path = tmp_dir / "baseline.json"
            baseline_path.write_text(json.dumps(_make_baseline()), encoding="utf-8")
            baseline = parity.load_baseline(baseline_path)

            win_expected = parity.expected_counts_for_platform(baseline, "windows")
            win_list = tmp_dir / "win.txt"
            win_list.write_text(_render_list(win_expected), encoding="utf-8")

            report_wrong_os = parity.evaluate_legacy_parity(
                root=tmp_dir,
                baseline_path=baseline_path,
                platform_arg="linux",
                list_file=win_list,
            )
            self.assertEqual(report_wrong_os["status"], "fail")
            self.assertEqual(report_wrong_os["violation_count"], 2)

            report_missing_solari = parity.evaluate_legacy_parity(
                root=tmp_dir,
                baseline_path=baseline_path,
                platform_arg="windows",
                list_file=win_list,
                include_solari=True,
            )
            self.assertEqual(report_missing_solari["status"], "fail")
            self.assertEqual(
                report_missing_solari["missing"],
                [{"name": "solari::scene_test", "expected_count": 2, "observed_count": 0}],
            )

    def test_rejects_malformed_list_and_corrupted_baseline_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            baseline_path = tmp_dir / "baseline.json"
            baseline_path.write_text(json.dumps(_make_baseline()), encoding="utf-8")

            no_footer = tmp_dir / "no_footer.txt"
            no_footer.write_text("alpha::unit: test\n", encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.evaluate_legacy_parity(
                    root=tmp_dir,
                    baseline_path=baseline_path,
                    platform_arg="linux",
                    list_file=no_footer,
                )

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = parity.main(
                    [
                        "--root",
                        str(tmp_dir),
                        "--baseline",
                        str(baseline_path),
                        "--platform",
                        "linux",
                        "--list-file",
                        str(no_footer),
                        "--json",
                    ]
                )
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(stdout.getvalue())["status"], "error")

            # Overlapping buckets in baseline manifest must raise BaselineError.
            overlap_path = tmp_dir / "overlap.json"
            overlap_data = _make_baseline(
                common={"dup::name": 1},
                windows_only={"dup::name": 1},
            )
            overlap_path.write_text(json.dumps(overlap_data), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_baseline(overlap_path)

            # Summary mismatch must raise BaselineError.
            bad_summary_path = tmp_dir / "bad_summary.json"
            bad_summary_data = _make_baseline(summary_overrides={"common_total_entries": 999})
            bad_summary_path.write_text(json.dumps(bad_summary_data), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_baseline(bad_summary_path)

    def test_cargo_subprocess_failure_returns_exit_code_2(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, baseline_path, _ = _qualified_fixture(root)
            fake_proc = subprocess.CompletedProcess(args=["cargo"], returncode=101, stdout="", stderr="linker error")
            with mock.patch.object(parity.subprocess, "run", return_value=fake_proc), contextlib.redirect_stderr(io.StringIO()) as stderr:
                code = parity.main(["--root", str(root), "--legacy-baseline", str(legacy_path), "--baseline", str(baseline_path)])
            self.assertEqual(code, 2)
            self.assertIn("failed with exit code 101", stderr.getvalue())

    def test_target_swap_empty_harness_and_repeated_parity(self):
        a = _target("a", {"same": 1})
        b = _target("b", {"same": 1})
        c = _target("c", {"same": 1})
        empty = _target("empty", {})
        expected = [a, b, empty]
        for observed, status in (([b,c,empty], "fail"), (expected,"pass"), ([a,b],"fail"), ([b,c,empty],"fail")):
            report = parity.qualified_report(expected, observed)
            self.assertEqual(report["status"], status)
        report = parity.qualified_report(expected, [b,c,empty])
        self.assertEqual(report["missing"][0]["target"], "a")
        self.assertEqual(report["unexpected"][0]["target"], "c")
        with self.assertRaises(parity.BaselineError):
            parity.parse_targets([a,a])

    def test_strict_footer_counts_and_incomplete_sections(self):
        invalid = ["one: test\n2 tests, 0 benchmarks\n", "one: test\n1 test, 1 benchmark\n",
                   "0 tests, 0 benchmarks\none: test\n", "noise\n0 tests, 0 benchmarks\n",
                   "one: test\n0 tests, 0 benchmarks\n", "one: test\n1 test, 0 benchmarks\n0 tests, 0 benchmarks\n"]
        for text in invalid:
            with self.subTest(text=text), self.assertRaises(parity.BaselineError):
                parity.parse_cargo_list_output(text, single_target=True)
        self.assertEqual(parity.parse_cargo_list_output("0 tests, 0 benchmarks\n", single_target=True), (Counter(), Counter()))
        self.assertEqual(parity.parse_cargo_list_output("one: test\n1 test, 0 benchmarks\n0 tests, 0 benchmarks\n")[0], {"one":1})
        self.assertEqual(parity.parse_cargo_list_output("bench: benchmark\n0 tests, 1 benchmark\n", single_target=True)[1], {"bench":1})

    def test_qualified_baseline_capture_merge_and_legacy_rejection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, baseline_path, configs = _qualified_fixture(root)
            legacy = parity.load_baseline(legacy_path)
            captures = []
            for config, doc in configs.items():
                path = root / (config.replace(":", "_") + ".json")
                path.write_text(json.dumps(doc), encoding="utf-8")
                captures.append(path)
            output = root / "merged.json"
            with self.assertRaises(parity.BaselineError):
                parity.merge_captures(captures[:-1], output, legacy)
            self.assertFalse(output.exists())
            parity.merge_captures(captures, output, legacy)
            self.assertEqual(parity.load_qualified(output, legacy), configs)
            with self.assertRaises(FileExistsError):
                parity.merge_captures(captures, output, legacy)
            with self.assertRaises(parity.BaselineError):
                parity.load_qualified(legacy_path, legacy)
            bad = root / "flat.txt"
            bad.write_text("0 tests, 0 benchmarks\n", encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()) as out:
                code = parity.main(["--root", str(root), "--baseline", str(baseline_path), "--legacy-baseline", str(legacy_path), "--list-file", str(bad), "--json"])
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(out.getvalue())["status"], "error")
            # The original name/count inventory must reconcile across every capture.
            configs["linux:default"]["targets"][0]["listing"] = "0 tests, 0 benchmarks\n"
            baseline_path.write_text(json.dumps(_baseline_doc(configs)), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_qualified(baseline_path, legacy)

    def test_c5_scene_index_aliases_preserve_target_feature_and_count(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)

        old_prefix = "viewport::api::scene_index::"
        new_prefix = "viewport::scene::index::"
        expected_targets = {
            ("Cargo.toml", ("bin",), "usdview"),
            ("Cargo.toml", ("lib",), "usdview"),
        }
        c5_migrations = [
            migration for migration in migrations if "BOV-C5" in migration["evidence"]
        ]
        self.assertEqual(len(c5_migrations), 16)
        for migration in c5_migrations:
            self.assertTrue(migration["old_name"].startswith(old_prefix))
            self.assertEqual(
                migration["new_name"],
                new_prefix + migration["old_name"][len(old_prefix):],
            )
            self.assertEqual(set(migration["targets"]), expected_targets)
            self.assertEqual(migration["configurations"], frozenset(parity.CONFIGS))

        for configuration in sorted(parity.CONFIGS):
            _, original_tests, benchmarks = parity.parse_targets(
                configs[configuration]["targets"]
            )
            self.assertFalse(benchmarks)
            old_entries = {
                key: count
                for key, count in original_tests.items()
                if key[-1].startswith(old_prefix)
            }
            self.assertEqual(len({key[-1] for key in old_entries}), 16)
            expected = {
                (*key[:3], new_prefix + key[-1][len(old_prefix):]): count
                for key, count in old_entries.items()
            }
            migrated, _ = parity.apply_identity_migrations(
                original_tests, migrations, configuration
            )
            actual = {
                key: count
                for key, count in migrated.items()
                if key[-1].startswith(new_prefix)
            }
            self.assertEqual(actual, expected)
            self.assertEqual(sum(actual.values()), 32)

    def test_bov_c6_stage_migrations_preserve_target_feature_and_count(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)

        prior_aliases = (
            (
                "viewport::app::project_activation::",
                "viewport::app::project::activation::",
            ),
            (
                "viewport::app::project_stage::project_runtime_authority::",
                "viewport::app::project::runtime_authority::",
            ),
            (
                "viewport::app::project_stage::",
                "viewport::app::project::stage::",
            ),
            # BOR-C7 flattened this pinned identity from
            # project_activation::scene_presentation under lifecycle.
            (
                "viewport::session::lifecycle::project_activation::scene_presentation::",
                "viewport::session::lifecycle::scene_presentation::",
            ),
        )
        c6_prefix_moves = (
            (
                "viewport::app::project::activation::",
                "viewport::stage::project::activation::",
            ),
            (
                "viewport::app::project::runtime_authority::",
                "viewport::stage::project::runtime_authority::",
            ),
            (
                "viewport::app::project::stage::",
                "viewport::stage::project::live_stage_runtime::",
            ),
            ("viewport::session::", "viewport::stage::"),
        )

        def c6_destination(name: str) -> str | None:
            c5_name = name
            for old_prefix, new_prefix in prior_aliases:
                if c5_name.startswith(old_prefix):
                    c5_name = new_prefix + c5_name[len(old_prefix):]
                    break
            for old_prefix, new_prefix in c6_prefix_moves:
                if c5_name.startswith(old_prefix):
                    return new_prefix + c5_name[len(old_prefix):]
            return None

        expected_targets = {
            ("Cargo.toml", ("bin",), "usdview"),
            ("Cargo.toml", ("lib",), "usdview"),
        }
        c6_migrations = [
            migration for migration in migrations if "BOV-C6" in migration["evidence"]
        ]
        self.assertEqual(len(c6_migrations), 38)
        expected_names = set()
        for migration in c6_migrations:
            destination = c6_destination(migration["old_name"])
            self.assertIsNotNone(destination)
            self.assertEqual(migration["new_name"], destination)
            self.assertEqual(set(migration["targets"]), expected_targets)
            self.assertEqual(migration["configurations"], frozenset(parity.CONFIGS))
            expected_names.add(migration["new_name"])
        self.assertEqual(len(expected_names), 38)

        for configuration in sorted(parity.CONFIGS):
            _, original_tests, benchmarks = parity.parse_targets(
                configs[configuration]["targets"]
            )
            self.assertFalse(benchmarks)
            old_entries = {
                key: count
                for key, count in original_tests.items()
                if c6_destination(key[-1]) is not None
            }
            self.assertEqual(len({key[-1] for key in old_entries}), 38)
            self.assertEqual(
                {key[-1] for key in old_entries},
                {migration["old_name"] for migration in c6_migrations},
            )
            self.assertEqual(len(old_entries), 76)
            expected = {
                (*key[:3], c6_destination(key[-1])): count
                for key, count in old_entries.items()
            }
            migrated, _ = parity.apply_identity_migrations(
                original_tests, migrations, configuration
            )
            actual = {
                key: count
                for key, count in migrated.items()
                if key[-1] in expected_names
            }
            self.assertEqual(actual, expected)
            self.assertEqual(sum(actual.values()), 76)

    def test_bov_c7_bim_semantic_migrations_preserve_target_feature_and_count(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)

        # BOR-C7 already composed these three pinned identities into the
        # physical scene/classification/plan module before BOV-C7 moved it.
        prior_aliases = (
            (
                "viewport::scene::classification_color_plan::",
                "viewport::scene::classification_color::plan::",
            ),
        )
        c7_prefix_moves = (
            (
                "viewport::api::bim_provenance::",
                "viewport::bim::provenance::",
            ),
            (
                "viewport::app::sync::",
                "viewport::semantic::sync::remote::",
            ),
            (
                "viewport::bim::classification_color::",
                "viewport::bim::classification::color::",
            ),
            (
                "viewport::bim::diff::",
                "viewport::bim::comparison::",
            ),
            (
                "viewport::scene::classification_color::",
                "viewport::bim::classification::presentation::",
            ),
            (
                "viewport::scene::diff::",
                "viewport::bim::comparison::visualization::",
            ),
            (
                "viewport::scene::ghost::",
                "viewport::bim::comparison::ghost::",
            ),
        )

        def c7_destination(name: str) -> str | None:
            c6_name = name
            for old_prefix, new_prefix in prior_aliases:
                if c6_name.startswith(old_prefix):
                    c6_name = new_prefix + c6_name[len(old_prefix):]
                    break
            for old_prefix, new_prefix in c7_prefix_moves:
                if c6_name.startswith(old_prefix):
                    return new_prefix + c6_name[len(old_prefix):]
            return None

        expected_targets = {
            ("Cargo.toml", ("bin",), "usdview"),
            ("Cargo.toml", ("lib",), "usdview"),
        }
        c7_migrations = [
            migration for migration in migrations if "BOV-C7" in migration["evidence"]
        ]
        self.assertEqual(len(c7_migrations), 22)
        expected_names = set()
        for migration in c7_migrations:
            destination = c7_destination(migration["old_name"])
            self.assertIsNotNone(destination)
            self.assertEqual(migration["new_name"], destination)
            self.assertEqual(set(migration["targets"]), expected_targets)
            self.assertEqual(migration["configurations"], frozenset(parity.CONFIGS))
            expected_names.add(migration["new_name"])
        self.assertEqual(len(expected_names), 22)

        for configuration in sorted(parity.CONFIGS):
            _, original_tests, benchmarks = parity.parse_targets(
                configs[configuration]["targets"]
            )
            self.assertFalse(benchmarks)
            old_entries = {
                key: count
                for key, count in original_tests.items()
                if c7_destination(key[-1]) is not None
            }
            self.assertEqual(len({key[-1] for key in old_entries}), 22)
            self.assertEqual(
                {key[-1] for key in old_entries},
                {migration["old_name"] for migration in c7_migrations},
            )
            self.assertEqual(len(old_entries), 44)
            expected = {
                (*key[:3], c7_destination(key[-1])): count
                for key, count in old_entries.items()
            }
            migrated, _ = parity.apply_identity_migrations(
                original_tests, migrations, configuration
            )
            actual = {
                key: count
                for key, count in migrated.items()
                if key[-1] in expected_names
            }
            self.assertEqual(actual, expected)
            self.assertEqual(sum(actual.values()), 44)

    def test_windows_executes_every_target_qualified_occurrence_and_rejects_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy = parity.load_baseline(_qualified_fixture(root)[0])
            name = "win_only::path_case"
            configs = {"windows:default": _envelope("windows:default", [_target("a", {name:1}), _target("b", {name:1})])}
            exes = {("Cargo.toml", ("lib",), "a"): (root/"a", root), ("Cargo.toml", ("lib",), "b"): (root/"b", root)}
            good = f"running 1 test\ntest {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            with mock.patch.object(parity, "checked_run", return_value=good) as run:
                receipts = parity.execute_windows(configs, legacy, exes)
                self.assertEqual(len(receipts),2)
                self.assertEqual(run.call_count,2)
                self.assertEqual({call.args[0][0] for call in run.call_args_list}, {str(root/"a"), str(root/"b")})
            with mock.patch.object(parity, "checked_run", return_value="running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored;\n"):
                with self.assertRaises(parity.BaselineError): parity.execute_windows(configs, legacy, exes)
            with self.assertRaises(parity.BaselineError): parity.execute_windows(configs, legacy, {})

    def test_windows_execution_runs_the_mapped_windows_only_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy = parity.load_baseline(_qualified_fixture(root)[0])
            old_name = "win_only::path_case"
            new_name = "win_only::relocated_path_case"
            targets = [
                {"manifest": "Cargo.toml", "kind": ["bin"], "target": "app"},
                {"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"},
            ]
            counts = {"alpha::unit": 2, "beta::contract": 1, old_name: 1}
            configs = {
                "windows:default": _envelope(
                    "windows:default",
                    [
                        _target("app", {"alpha::unit": 2, "beta::contract": 1, old_name: 1}),
                        {"manifest": "Cargo.toml", "kind": ["bin"], "target": "app",
                         "listing": _render_list(Counter({"alpha::unit": 2, "beta::contract": 1, old_name: 1}))},
                    ],
                )
            }
            migrations = [
                {
                    "old_name": old_name,
                    "new_name": new_name,
                    "targets": (
                        ("Cargo.toml", ("bin",), "app"),
                        ("Cargo.toml", ("lib",), "app"),
                    ),
                    "configurations": frozenset({"windows:default", "windows:solari"}),
                    "evidence": ("BOR-C2",),
                }
            ]
            exes = {
                ("Cargo.toml", ("bin",), "app"): (root / "app-bin", root),
                ("Cargo.toml", ("lib",), "app"): (root / "app-lib", root),
            }
            good = (
                f"running 1 test\ntest {new_name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )
            with mock.patch.object(parity, "checked_run", return_value=good) as run:
                receipts = parity.execute_windows(
                    configs,
                    legacy,
                    exes,
                    identity_migrations=migrations,
                )
            self.assertEqual(len(receipts), 2)
            self.assertEqual(
                {call.args[0][2] for call in run.call_args_list},
                {new_name},
            )

            old_name_output = (
                f"running 1 test\ntest {old_name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )
            with mock.patch.object(parity, "checked_run", return_value=old_name_output):
                with self.assertRaises(parity.BaselineError):
                    parity.execute_windows(
                        configs,
                        legacy,
                        exes,
                        identity_migrations=migrations,
                    )

    def test_checked_in_windows_only_migration_is_selected_and_executed(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)

        old_name = (
            "project::cache_warmer::legacy_migration_tests::"
            "windows_legacy_project_root_cache_is_blocked_without_changing_legacy_state"
        )
        new_name = (
            "project::cache::warming::legacy_migration_tests::"
            "windows_legacy_project_root_cache_is_blocked_without_changing_legacy_state"
        )
        required = parity.windows_required(configs, legacy, identity_migrations=migrations)
        matching_keys = [key for key in required if key[-1] == new_name]
        self.assertEqual(len(matching_keys), 2)
        self.assertNotIn(old_name, [key[-1] for key in required])

        executables = {key[:3]: (Path("cargo-test.exe"), REPO_ROOT) for key in required}

        def successful_exact_run(command, _cwd, *, env):
            del env
            name = command[2]
            return (
                "running 1 test\n"
                f"test {name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )

        with mock.patch.object(parity, "checked_run", side_effect=successful_exact_run) as run:
            receipts = parity.execute_windows(
                configs,
                legacy,
                executables,
                identity_migrations=migrations,
            )
        executed_names = [call.args[0][2] for call in run.call_args_list]
        self.assertIn(new_name, executed_names)
        self.assertNotIn(old_name, executed_names)
        self.assertTrue(set(matching_keys).issubset(receipts))

        def stale_identity_run(command, _cwd, *, env):
            del env
            name = old_name if command[2] == new_name else command[2]
            return (
                "running 1 test\n"
                f"test {name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )

        with mock.patch.object(parity, "checked_run", side_effect=stale_identity_run):
            with self.assertRaises(parity.BaselineError):
                parity.execute_windows(
                    configs,
                    legacy,
                    executables,
                    identity_migrations=migrations,
                )

    def test_execute_windows_cli_uses_the_checked_in_windows_only_migration(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)
        old_name = (
            "project::cache_warmer::legacy_migration_tests::"
            "windows_legacy_project_root_cache_is_blocked_without_changing_legacy_state"
        )
        new_name = (
            "project::cache::warming::legacy_migration_tests::"
            "windows_legacy_project_root_cache_is_blocked_without_changing_legacy_state"
        )

        target_ids, original_tests, original_benchmarks = parity.parse_targets(
            configs["windows:default"]["targets"]
        )
        mapped_tests, _ = parity.apply_identity_migrations(
            original_tests, migrations, "windows:default"
        )
        observed_targets = []
        for manifest, kinds, target in sorted(target_ids):
            target_id = (manifest, kinds, target)
            tests = Counter(
                {key[-1]: count for key, count in mapped_tests.items() if key[:3] == target_id}
            )
            benchmarks = [
                key[-1] for key in original_benchmarks if key[:3] == target_id
            ]
            observed_targets.append(
                {
                    "manifest": manifest,
                    "kind": list(kinds),
                    "target": target,
                    "listing": _render_list(tests, benchmarks),
                }
            )

        required = parity.windows_required(configs, legacy, identity_migrations=migrations)
        executables = {
            key[:3]: (Path("cargo-test.exe"), REPO_ROOT) for key in required
        }

        def live_capture(_root, include_solari):
            self.assertFalse(include_solari)
            return observed_targets, executables

        def successful_exact_run(command, _cwd, *, env):
            del env
            name = command[2]
            return (
                "running 1 test\n"
                f"test {name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )

        success_output = io.StringIO()
        with (
            mock.patch.object(parity, "capture", side_effect=live_capture),
            mock.patch.object(parity, "harness_env", return_value={}),
            mock.patch.object(parity, "checked_run", side_effect=successful_exact_run) as run,
            contextlib.redirect_stdout(success_output),
        ):
            result = parity.main(
                ["--root", str(REPO_ROOT), "--platform", "windows", "--execute-windows", "--json"]
            )
        self.assertEqual(result, 0)
        receipt = json.loads(success_output.getvalue())
        self.assertEqual(receipt["status"], "pass")
        executed_names = [key[-1] for key in receipt["executed_windows_tests"]]
        self.assertIn(new_name, executed_names)
        self.assertNotIn(old_name, executed_names)
        self.assertIn(new_name, [call.args[0][2] for call in run.call_args_list])

        def stale_identity_run(command, _cwd, *, env):
            del env
            name = old_name if command[2] == new_name else command[2]
            return (
                "running 1 test\n"
                f"test {name} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )

        failure_output = io.StringIO()
        with (
            mock.patch.object(parity, "capture", side_effect=live_capture),
            mock.patch.object(parity, "harness_env", return_value={}),
            mock.patch.object(parity, "checked_run", side_effect=stale_identity_run),
            contextlib.redirect_stdout(failure_output),
        ):
            result = parity.main(
                ["--root", str(REPO_ROOT), "--platform", "windows", "--execute-windows", "--json"]
            )
        self.assertEqual(result, 2)
        self.assertIn("Windows test was not demonstrably executed", json.loads(failure_output.getvalue())["error"])

    def test_cargo_artifacts_bind_by_metadata_not_executable_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            pkg = root / "Cargo.toml"
            src = root / "src.rs"
            exe_a, exe_b = root/"hash1", root/"hash2"
            exe_a.touch(); exe_b.touch()
            ta = {"name":"a", "kind":["lib"], "src_path":str(src)}
            tb = {"name":"b", "kind":["bin"], "src_path":str(src)}
            meta = {"workspace_members":["pkg"], "packages":[{"id":"pkg", "manifest_path":str(pkg), "targets":[ta,tb]}]}
            def artifact(t, exe):
                return {"reason":"compiler-artifact","package_id":"pkg","target":t,"profile":{"test":True},"executable":str(exe)}
            messages = [artifact(tb,exe_b), artifact(ta,exe_a), {"reason":"build-finished","success":True}]
            with mock.patch.object(parity, "checked_run", side_effect=[json.dumps(meta), "\n".join(map(json.dumps,messages))]):
                result = parity.discover_executables(root, False)
            self.assertEqual(result[("Cargo.toml",("lib",),"a")][0], exe_a)
            self.assertEqual(result[("Cargo.toml",("bin",),"b")][0], exe_b)
            messages.insert(0,artifact(ta,exe_a))
            with mock.patch.object(parity, "checked_run", side_effect=[json.dumps(meta), "\n".join(map(json.dumps,messages))]):
                with self.assertRaises(parity.BaselineError): parity.discover_executables(root, False)

    def test_baseline_capture_rejects_wrong_head_or_dirty_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with mock.patch.object(parity, "checked_run", return_value="wrong"):
                with self.assertRaises(parity.BaselineError): parity.assert_baseline_checkout(root)
            with mock.patch.object(parity, "checked_run", side_effect=[parity.BASELINE_COMMIT, " M src/lib.rs"]):
                with self.assertRaises(parity.BaselineError): parity.assert_baseline_checkout(root)

    def test_harness_env_includes_target_libdir_and_preserves_existing_vars(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            deps_dir = root / "target" / "debug" / "deps"
            target_libdir = root / "rustlib" / "lib"
            deps_dir.mkdir(parents=True)
            target_libdir.mkdir(parents=True)
            exe = deps_dir / "usd_macro.exe"
            exe.touch()
            existing_dir = str(root / "existing")
            fake_rustc = subprocess.CompletedProcess(
                args=["rustc", "--print", "target-libdir"],
                returncode=0,
                stdout=f"{target_libdir}\n",
                stderr="",
            )
            parity._rustc_target_libdir.cache_clear()
            try:
                with (
                    mock.patch.dict(
                        parity.os.environ,
                        {
                            "PATH": f"{existing_dir}{parity.os.pathsep}{target_libdir}",
                            "LD_LIBRARY_PATH": existing_dir,
                            "DYLD_FALLBACK_LIBRARY_PATH": existing_dir,
                        },
                        clear=False,
                    ),
                    mock.patch.object(parity.subprocess, "run", return_value=fake_rustc),
                ):
                    env = parity.harness_env(exe)
                for var in ("PATH", "LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"):
                    entries = env[var].split(parity.os.pathsep)
                    self.assertEqual(
                        entries[:3],
                        [str(deps_dir), str(deps_dir.parent), str(target_libdir)],
                    )
                    self.assertIn(existing_dir, entries[3:])
                    self.assertEqual(entries.count(str(target_libdir)), 1)

                parity._rustc_target_libdir.cache_clear()
                with mock.patch.object(parity.subprocess, "run", side_effect=OSError("no rustc")):
                    fallback_env = parity.harness_env(exe)
                self.assertEqual(
                    fallback_env["PATH"].split(parity.os.pathsep)[:2],
                    [str(deps_dir), str(deps_dir.parent)],
                )
            finally:
                parity._rustc_target_libdir.cache_clear()

    def test_exact_identity_migrations_are_target_and_configuration_scoped(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, baseline_path, configs = _qualified_fixture(root)
            migration_path = root / "migrations.json"
            migration = {
                "schema_version": 1,
                "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                "baseline_commit": parity.BASELINE_COMMIT,
                "entries": [
                    {
                        "old_name": "alpha::unit",
                        "new_name": "alpha::relocated",
                        "targets": [
                            {"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"}
                        ],
                        "configurations": sorted(parity.CONFIGS),
                        "evidence": ["BOV-C4"],
                    }
                ],
            }
            migration_path.write_text(json.dumps(migration), encoding="utf-8")
            migrations = parity.load_identity_migrations(migration_path, configs)

            expected = [
                _target("app", {"alpha::unit": 2, "beta::contract": 1}),
                _target("another-target", {"alpha::unit": 1}),
            ]
            observed = [
                _target("app", {"alpha::relocated": 2, "beta::contract": 1}),
                _target("another-target", {"alpha::unit": 1}),
            ]
            report = parity.qualified_report(
                expected,
                observed,
                identity_migrations=migrations,
                configuration="linux:default",
            )
            self.assertEqual(report["status"], "pass")
            self.assertEqual(report["identity_migrations_applied"], 1)

            missing_after_move = parity.qualified_report(
                expected,
                [_target("app", {"beta::contract": 1}), expected[1]],
                identity_migrations=migrations,
                configuration="linux:default",
            )
            self.assertEqual(missing_after_move["missing"][0]["name"], "alpha::relocated")
            self.assertEqual(missing_after_move["missing"][0]["expected_count"], 2)

            old_identity_retained = parity.qualified_report(
                expected,
                [_target("app", {"alpha::relocated": 2, "alpha::unit": 1, "beta::contract": 1}), observed[1]],
                identity_migrations=migrations,
                configuration="linux:default",
            )
            self.assertEqual(old_identity_retained["unexpected"][0]["name"], "alpha::unit")

    def test_identity_migration_loader_rejects_wrong_presence_and_collisions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, _, configs = _qualified_fixture(root)
            migration_path = root / "migrations.json"
            base_entry = {
                "old_name": "alpha::unit",
                "new_name": "alpha::relocated",
                "targets": [
                    {"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"}
                ],
                "configurations": sorted(parity.CONFIGS),
                "evidence": ["BOV-C4"],
            }

            wrong_presence = json.loads(json.dumps(base_entry))
            wrong_presence["configurations"] = ["windows:default"]
            migration = {
                "schema_version": 1,
                "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                "baseline_commit": parity.BASELINE_COMMIT,
                "entries": [wrong_presence],
            }
            migration_path.write_text(json.dumps(migration), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

            collision = json.loads(json.dumps(base_entry))
            collision["new_name"] = "beta::contract"
            migration["entries"] = [collision]
            migration_path.write_text(json.dumps(migration), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

            migration["entries"] = [base_entry, json.loads(json.dumps(base_entry))]
            migration_path.write_text(json.dumps(migration), encoding="utf-8")
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

    def test_solari_identity_migrations_do_not_change_default_expectations(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy_path, _, configs = _qualified_fixture(root)
            migration_path = root / "migrations.json"
            solari_configs = sorted(config for config in parity.CONFIGS if config.endswith(":solari"))
            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [
                            {
                                "old_name": "solari::scene_test",
                                "new_name": "solari::relocated_scene_test",
                                "targets": [
                                    {"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"}
                                ],
                                "configurations": solari_configs,
                                "evidence": ["BOV-C4"],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            migrations = parity.load_identity_migrations(migration_path, configs)
            solari_expected = configs["linux:solari"]["targets"]
            solari_observed = [
                _target("app", {"alpha::unit": 2, "beta::contract": 1,
                                 "unix_only::symlink_case": 2,
                                 "solari::relocated_scene_test": 2})
            ]
            solari_report = parity.qualified_report(
                solari_expected,
                solari_observed,
                identity_migrations=migrations,
                configuration="linux:solari",
            )
            self.assertEqual(solari_report["status"], "pass")

            default_report = parity.qualified_report(
                configs["linux:default"]["targets"],
                [
                    _target(
                        "app",
                        {
                            "alpha::unit": 2,
                            "beta::contract": 1,
                            "unix_only::symlink_case": 2,
                            "solari::relocated_scene_test": 1,
                        },
                    )
                ],
                identity_migrations=migrations,
                configuration="linux:default",
            )
            self.assertEqual(default_report["status"], "fail")
            self.assertEqual(
                default_report["unexpected"][0]["name"],
                "solari::relocated_scene_test",
            )

    def test_identity_migration_additions_apply_and_reject_collisions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _, _, configs = _qualified_fixture(root)
            migration_path = root / "migrations.json"
            target = [{"manifest": "Cargo.toml", "kind": ["lib"], "target": "app"}]
            all_configs = sorted(parity.CONFIGS)
            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [],
                        "additions": [
                            {
                                "name": "ported::dormant_contract",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["NEM-CLN-C1"],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            migrations = parity.load_identity_migrations(migration_path, configs)
            self.assertEqual(len(migrations.additions), 1)
            report = parity.qualified_report(
                [_target("app", {"alpha::unit": 2, "beta::contract": 1})],
                [
                    _target(
                        "app",
                        {
                            "alpha::unit": 2,
                            "beta::contract": 1,
                            "ported::dormant_contract": 1,
                        },
                    )
                ],
                identity_migrations=migrations,
                configuration="windows:default",
            )
            self.assertEqual(report["status"], "pass")
            self.assertEqual(report["identity_migrations_applied"], 1)

            # Reject colliding with pinned baseline identity
            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [],
                        "additions": [
                            {
                                "name": "alpha::unit",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["NEM-CLN-C1"],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

            # Reject colliding with a rename migration destination
            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [
                            {
                                "old_name": "alpha::unit",
                                "new_name": "alpha::relocated",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["BOV-C4"],
                            }
                        ],
                        "additions": [
                            {
                                "name": "alpha::relocated",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["NEM-CLN-C1"],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

            migration_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "baseline_checkpoint": parity.BASELINE_CHECKPOINT,
                        "baseline_commit": parity.BASELINE_COMMIT,
                        "entries": [],
                        "additions": [
                            {
                                "name": "ported::dup",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["NEM-CLN-C1"],
                            },
                            {
                                "name": "ported::dup",
                                "targets": target,
                                "configurations": all_configs,
                                "evidence": ["NEM-CLN-C1"],
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(parity.BaselineError):
                parity.load_identity_migrations(migration_path, configs)

    def test_checked_in_additions_match_current_configs(self):
        legacy = parity.load_baseline(CHECKED_IN_BASELINE)
        baseline_path = REPO_ROOT / "scripts" / "test_enumeration_qualified_baseline.json"
        configs = parity.load_qualified(baseline_path, legacy)
        migrations_path = REPO_ROOT / "scripts" / parity.IDENTITY_MIGRATIONS_FILE
        migrations = parity.load_identity_migrations(migrations_path, configs)
        self.assertEqual(len(migrations.additions), 60)
        counts = {}
        for addition in migrations.additions:
            counts[addition["evidence"]] = counts.get(addition["evidence"], 0) + 1
        self.assertEqual(
            counts,
            {
                ("NEM-CLN-C1",): 21,
                ("UP-C01",): 4,
                ("UP-C02",): 19,
                ("UP-C03",): 1,
                ("UP-C04",): 2,
                ("UP-C06",): 13,
            },
        )


if __name__ == "__main__":
    unittest.main()
