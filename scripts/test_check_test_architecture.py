#!/usr/bin/env python3
"""Unit and adversarial fixture tests for scripts/check_test_architecture.py."""

from __future__ import annotations

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_test_architecture as cta


def _base_policy() -> dict:
    return {
        "test_support_crates": ["usd_support"],
        "forbidden_test_file_names": [
            "tests.rs",
            "test.rs",
            "test_support.rs",
            "test_helpers.rs",
            "test_fixtures.rs",
        ],
        "forbidden_test_attr_suffixes": ["test", "rstest", "test_case"],
        "reviewed_path_only_attachments": [],
        "generated_sources": [],
        "exceptional_production_fixtures": [],
    }


def _write_workspace(
    root: Path,
    *,
    members: list[str] | None = None,
    ws_deps: dict[str, str] | None = None,
    policy: dict | None = None,
) -> None:
    members = members or ["crates/app", "tests/support/usd_support"]
    lines = ["[workspace]", f"members = {json.dumps(members)}", 'resolver = "2"', ""]
    if ws_deps:
        lines.append("[workspace.dependencies]")
        for k, v in ws_deps.items():
            lines.append(f"{k} = {v}")
        lines.append("")
    (root / "Cargo.toml").write_text("\n".join(lines), encoding="utf-8")

    # Minimal support crate
    sup_dir = root / "tests" / "support" / "usd_support"
    sup_dir.mkdir(parents=True, exist_ok=True)
    (sup_dir / "Cargo.toml").write_text(
        '[package]\nname = "usd_support"\nversion = "0.1.0"\nedition = "2021"\npublish = false\n\n[lib]\npath = "lib.rs"\n',
        encoding="utf-8",
    )
    (sup_dir / "lib.rs").write_text("pub fn helper() {}\n", encoding="utf-8")

    # Minimal app crate
    app_dir = root / "crates" / "app"
    (app_dir / "src").mkdir(parents=True, exist_ok=True)
    (app_dir / "tests" / "unit").mkdir(parents=True, exist_ok=True)
    (app_dir / "Cargo.toml").write_text(
        '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n[dev-dependencies]\nusd_support = { path = "../../tests/support/usd_support" }\n',
        encoding="utf-8",
    )
    (app_dir / "src" / "lib.rs").write_text("pub fn run() {}\n", encoding="utf-8")

    (root / "scripts").mkdir(parents=True, exist_ok=True)
    pol = policy if policy is not None else _base_policy()
    (root / "scripts" / "test_architecture_policy.json").write_text(
        json.dumps(pol, indent=2), encoding="utf-8"
    )


def _add_valid_minimum(root: Path) -> None:
    """Give a positive fixture one real external unit test and reviewed attachment."""
    owner = root / "crates/app/src/lib.rs"
    target = root / "crates/app/tests/unit/minimum_tests.rs"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("#[test]\nfn minimum_test() {}\n", encoding="utf-8")
    with owner.open("a", encoding="utf-8") as fh:
        fh.write('\n#[cfg(test)]\n#[path = "../tests/unit/minimum_tests.rs"]\nmod minimum_tests;\n')
    policy_path = root / "scripts/test_architecture_policy.json"
    policy = json.loads(policy_path.read_text(encoding="utf-8"))
    policy["reviewed_path_only_attachments"].append({
        "owner_file": "crates/app/src/lib.rs", "module_path": "",
        "mod_name": "minimum_tests", "target": "crates/app/tests/unit/minimum_tests.rs",
    })
    policy_path.write_text(json.dumps(policy), encoding="utf-8")


def _codes(report: dict) -> set[str]:
    return {v["code"] for v in report["violations"]}


class TestCanonicalArchitectureGuard(unittest.TestCase):
    def test_clean_workspace_with_reviewed_path_attachment_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["reviewed_path_only_attachments"] = [
                {
                    "owner_file": "crates/app/src/lib.rs",
                    "module_path": "",
                    "mod_name": "tests",
                    "target": "crates/app/tests/unit/lib_tests.rs",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '/// Ignore comment `#[test]` and string "#[cfg(test)]"\n'
                'pub fn prod() -> &\'static str { "#[tokio::test]" }\n'
                "#[allow(dead_code)]\n"
                "#[cfg(test)]\n"
                '#[path = "../tests/unit/lib_tests.rs"]\n'
                "mod tests;\n",
                encoding="utf-8",
            )
            (root / "crates" / "app" / "tests" / "unit" / "lib_tests.rs").write_text(
                "#[test]\nfn it_works() {}\n", encoding="utf-8"
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

    def test_clean_production_helper_and_generated_source_pass(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["generated_sources"] = [
                {
                    "path": "assets/sample.txt",
                    "producer": "crates/app/build.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Build artifact asset outside src/",
                    "status": "approved",
                }
            ]
            pol["exceptional_production_fixtures"] = [
                {
                    "path": "crates/app/src/helpers.rs",
                    "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Production helper module",
                    "status": "approved",
                },
                {
                    "path": "crates/app/src/shader.wgsl",
                    "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Embedded production shader",
                    "status": "approved",
                },
            ]
            _write_workspace(root, policy=pol)
            (root / "assets").mkdir()
            (root / "assets" / "sample.txt").write_text("ok\n", encoding="utf-8")
            (root / "crates" / "app" / "build.rs").write_text(
                'fn main() { std::fs::write("../../assets/sample.txt", b"ok\\n").unwrap(); }\n', encoding="utf-8"
            )
            (root / "crates" / "app" / "src" / "helpers.rs").write_text("pub fn h() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "shader.wgsl").write_text("@vertex fn vs() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                'mod helpers;\nconst SHADER: &str = include_str!("shader.wgsl");\nconst GEN: &str = include_str!("../../../assets/sample.txt");\npub fn f() { helpers::h(); }\n',
                encoding="utf-8",
            )
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

    def test_rejects_removed_reviewed_seams_key_and_inline_cfg_test(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["reviewed_seams"] = [{"path": "crates/app/src/lib.rs"}]
            _write_workspace(root, policy=pol)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("forbidden-policy-key", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "#[cfg(test)]\nfn hook() {}\n#[cfg(not(test))]\nfn stub() {}\n",
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("forbidden-non-path-cfg-test", _codes(report))
            self.assertIn("forbidden-cfg-not-test", _codes(report))

    def test_f02_rejects_qualified_rstest_test_case_cfg_attr_cfg_macro_delimiters_and_raw_mod(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "#[::tokio::test]\nasync fn t1() {}\n"
                '#[cfg_attr(feature = "fixture_tests", rstest)]\nfn t2() {}\n'
                "#[test_case::test_case(1)]\nfn t2b(_: i32) {}\n"
                "#[cfg_attr(all(), cfg(test))]\nfn t3() {}\n"
                '#[cfg_attr(feature = "a", cfg_attr(feature = "b", ::tokio::r#test))]\nfn t4() {}\n'
                "#[cfg_attr(test, allow(dead_code))]\nfn t5() {}\n"
                "pub fn f() -> bool { cfg!(test) || cfg!{test} || cfg![test] || ::core::cfg!(test) }\n"
                "mod r#tests {}\n",
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("forbidden-test-attribute", codes)
            self.assertIn("forbidden-cfg-attr-nested-cfg-test", codes)
            self.assertIn("forbidden-cfg-attr-test", codes)
            self.assertIn("forbidden-cfg-macro-test", codes)
            self.assertIn("forbidden-inline-test-module", codes)

            # Assert all 4 cfg! delimiter/path forms fired
            cfg_macro_msgs = [v["message"] for v in report["violations"] if v["code"] == "forbidden-cfg-macro-test"]
            self.assertGreaterEqual(len(cfg_macro_msgs), 4, cfg_macro_msgs)

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "pub fn broken() -> bool { cfg!(test }\n", encoding="utf-8"
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("malformed-cfg-macro", _codes(report))

    def test_f02_strict_direct_cfg_test_and_path_provenance(self) -> None:
        # Reject compound cfg(any(test, ...)) and cfg(all(test, ...)) on external mod
        for expr in (
            '#[cfg(any(test, feature = "fixture_tests"))]',
            "#[cfg(all(test, unix))]",
        ):
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                pol = _base_policy()
                pol["reviewed_path_only_attachments"] = [
                    {
                        "owner_file": "crates/app/src/lib.rs",
                        "module_path": "",
                        "mod_name": "tests",
                        "target": "crates/app/tests/unit/lib_tests.rs",
                    }
                ]
                _write_workspace(root, policy=pol)
                (root / "crates" / "app" / "tests" / "unit" / "lib_tests.rs").write_text("", encoding="utf-8")
                (root / "crates" / "app" / "src" / "lib.rs").write_text(
                    f'{expr}\n#[path = "../tests/unit/lib_tests.rs"]\nmod tests;\n',
                    encoding="utf-8",
                )
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                self.assertIn("non-exclusive-cfg-test-attachment", _codes(report), expr)

        # Reject conditional cfg_attr(..., path = "..."), duplicate #[path], and extra non-lint attrs
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "tests" / "unit" / "lib_tests.rs").write_text("", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '#[cfg(test)]\n#[cfg_attr(feature = "alt", path = "../tests/unit/lib_tests.rs")]\nmod m1;\n'
                '#[cfg(test)]\n#[path = "../tests/unit/lib_tests.rs"]\n#[path = "../tests/unit/lib_tests.rs"]\nmod m2;\n'
                '#[cfg(test)]\n#[cfg(feature = "x")]\n#[path = "../tests/unit/lib_tests.rs"]\nmod m3;\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("conditional-path-on-test-mod", codes)
            self.assertIn("duplicate-path-attribute", codes)
            self.assertIn("forbidden-extra-attr-on-test-mod", codes)

    def test_f02_policy_weakened_minimums_rejected(self) -> None:
        # Empty test_support_crates
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["test_support_crates"] = []
            _write_workspace(root, policy=pol)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("policy-weakened-minimums", _codes(report))

        # Missing test_case suffix
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["forbidden_test_attr_suffixes"] = ["test", "rstest"]
            _write_workspace(root, policy=pol)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("policy-weakened-minimums", _codes(report))

        # Missing mandatory filename
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["forbidden_test_file_names"] = ["tests.rs"]
            _write_workspace(root, policy=pol)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("policy-weakened-minimums", _codes(report))

    def test_f02_fails_closed_on_bad_string_escape_and_unclosed_comment(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '#[cfg(test)]\n#[path = "../tests/unit\\q.rs"]\nmod tests;\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unsupported-rust-syntax", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "/* unclosed comment\npub fn x() {}\n", encoding="utf-8"
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unsupported-rust-syntax", _codes(report))

    def test_f03_inline_mod_path_resolving_into_src_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "shared.rs").write_text("pub fn s() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                'mod feature {\n    #[cfg(test)]\n    #[path = "../shared.rs"]\n    mod checks;\n}\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("cfg-test-path-inside-src", _codes(report))

    def test_f03_non_mod_rs_and_target_escape_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "shared.rs").write_text("", encoding="utf-8")
            # Inside non-mod.rs foo.rs, direct #[path = "shared.rs"] resolves relative to src/ -> inside src/!
            (root / "crates" / "app" / "src" / "foo.rs").write_text(
                '#[cfg(test)]\n#[path = "shared.rs"]\nmod checks;\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("cfg-test-path-inside-src", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "target" / "gen").mkdir(parents=True)
            (root / "target" / "gen" / "t.rs").write_text("", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '#[cfg(test)]\n#[path = "../../../target/gen/t.rs"]\nmod tests;\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("path-inside-target", _codes(report))

    def test_f03_empty_tests_dir_vs_empty_prod_dir(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "nested" / "tests").mkdir(parents=True)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("forbidden-tests-dir-in-src", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "nested" / "empty_prod").mkdir(parents=True)
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

    def test_f03_unreviewed_and_stale_reviewed_attachments_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["reviewed_path_only_attachments"] = [
                {
                    "owner_file": "crates/app/src/lib.rs",
                    "module_path": "",
                    "mod_name": "stale",
                    "target": "crates/app/tests/unit/lib_tests.rs",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "crates" / "app" / "tests" / "unit" / "lib_tests.rs").write_text("", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '#[cfg(test)]\n#[path = "../tests/unit/lib_tests.rs"]\nmod tests;\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("unreviewed-path-only-attachment", codes)
            self.assertIn("stale-reviewed-attachment", codes)

    def test_f03_fixture_classification_real_owner_shadowing_asset_macro_and_include_binding(self) -> None:
        # Real classified production fixture (src/fixtures.rs with owner mod + real consumer call) passes
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["exceptional_production_fixtures"] = [
                {
                    "path": "crates/app/src/fixtures.rs",
                    "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Production sample preset provider",
                    "status": "approved",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "crates" / "app" / "src" / "fixtures.rs").write_text(
                "pub fn default_preset() -> u32 { 7 }\n", encoding="utf-8"
            )
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "mod fixtures;\npub fn run() -> u32 { fixtures::default_preset() }\n", encoding="utf-8"
            )
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

        # Unequivocal test_fixtures.rs + unclassified fixtures.rs + unlisted include!
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "test_fixtures.rs").write_text("pub fn fake() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "fixtures.rs").write_text("pub fn fake2() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                'include!(concat!(env!("OUT_DIR"), "/gen.rs"));\n', encoding="utf-8"
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("forbidden-test-filename-in-src", codes)
            self.assertIn("unclassified-production-fixture", codes)
            self.assertIn("unclassified-generated-source", codes)

        # Shadowed local variable `let sample_data = 1;` + bare `pub mod sample_data;` fails consumer check
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["exceptional_production_fixtures"] = [
                {
                    "path": "crates/app/src/sample_data.rs",
                    "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Sneaky shadowed local variable bypass",
                    "status": "approved",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "crates" / "app" / "src" / "sample_data.rs").write_text("pub fn f() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "pub mod sample_data;\npub fn run() { let sample_data = 1; let _ = sample_data; }\n",
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("unreferenced-fixture-consumer", codes)
            self.assertIn("test-only-fixture-in-src", codes)

        # Wrong owner (doesn't bind sample_data.rs) fails unowned-production-fixture
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["exceptional_production_fixtures"] = [
                {
                    "path": "crates/app/src/sample_data.rs",
                    "owner": "crates/app/src/other.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Wrong owner test",
                    "status": "approved",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "crates" / "app" / "src" / "other.rs").write_text("pub fn o() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "sample_data.rs").write_text("pub fn f() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "mod other;\nmod sample_data;\npub fn run() { sample_data::f(); }\n", encoding="utf-8"
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unverified-fixture-owner", _codes(report))

        # Non-Rust asset plain string mention (not macro) + unapproved include!("../tests/...")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pol = _base_policy()
            pol["exceptional_production_fixtures"] = [
                {
                    "path": "crates/app/src/shader.wgsl",
                    "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Plain string should not satisfy asset macro check",
                    "status": "approved",
                }
            ]
            pol["generated_sources"] = [
                {
                    "path": "assets/gen.rs",
                    "producer": "crates/app/build.rs",
                    "consumers": ["crates/app/src/lib.rs"],
                    "reason": "Approved asset",
                    "status": "approved",
                }
            ]
            _write_workspace(root, policy=pol)
            (root / "assets").mkdir()
            (root / "assets" / "gen.rs").write_text("const X: u32 = 1;\n", encoding="utf-8")
            (root / "crates" / "app" / "build.rs").write_text("fn main() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "tests" / "unit" / "sneaky.rs").write_text("fn t() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "shader.wgsl").write_text("@vertex fn vs() {}\n", encoding="utf-8")
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                'const BAD_STR: &str = "shader.wgsl";\ninclude!("../tests/unit/sneaky.rs");\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("unreferenced-fixture-consumer", codes)
            self.assertIn("test-only-fixture-in-src", codes)
            self.assertIn("include-test-source-in-src", codes)

    def test_f04_support_src_dir_containment_grouped_use_qualified_path_and_cycles(self) -> None:
        # 1) Support crate with src/, missing [lib] path="lib.rs", empty & populated src/, cross-crate #[path] containment,
        #    qualified path `usd_support::helper()`, and grouped `use {std::fmt, sup_alias::s};`
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            members = ["crates/app", "crates/mid", "tests/support/usd_support", "tests/support/new_sup"]
            ws_deps = {
                "sup_alias": '{ package = "new_sup", path = "tests/support/new_sup" }',
            }
            _write_workspace(root, members=members, ws_deps=ws_deps)

            new_sup = root / "tests" / "support" / "new_sup"
            (new_sup / "src").mkdir(parents=True)
            (new_sup / "Cargo.toml").write_text(
                '[package]\nname = "new_sup"\nversion = "0.1.0"\nedition = "2021"\n\n[lib]\nname = "new_sup"\n',
                encoding="utf-8",
            )
            (new_sup / "src" / "lib.rs").write_text("#[test]\nfn sneaking_in_support_src() {}\n", encoding="utf-8")

            mid = root / "crates" / "mid"
            (mid / "src").mkdir(parents=True)
            (mid / "Cargo.toml").write_text(
                '[package]\nname = "mid"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nsup_alias = { workspace = true }\nusd_support = { path = "../../tests/support/usd_support" }\n',
                encoding="utf-8",
            )
            (mid / "src" / "lib.rs").write_text(
                "use {std::fmt, sup_alias::s};\npub fn run() { usd_support::helper(); let _ = fmt::Error; }\n",
                encoding="utf-8",
            )

            pol = json.loads((root / "scripts" / "test_architecture_policy.json").read_text(encoding="utf-8"))
            pol["reviewed_path_only_attachments"] = [
                {
                    "owner_file": "crates/app/src/lib.rs",
                    "module_path": "",
                    "mod_name": "sneaky_sup",
                    "target": "tests/support/new_sup/src/lib.rs",
                }
            ]
            (root / "scripts" / "test_architecture_policy.json").write_text(
                json.dumps(pol, indent=2), encoding="utf-8"
            )
            (root / "crates" / "app" / "Cargo.toml").write_text(
                '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nmid = { path = "../mid" }\n',
                encoding="utf-8",
            )
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                '#[cfg(test)]\n#[path = "../../../tests/support/new_sup/src/lib.rs"]\nmod sneaky_sup;\n',
                encoding="utf-8",
            )

            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            codes = _codes(report)
            self.assertIn("support-crate-src-dir", codes)
            self.assertIn("support-crate-missing-root-lib", codes)
            self.assertIn("forbidden-test-attribute", codes)
            self.assertIn("cfg-test-path-inside-src", codes)
            self.assertIn("support-crate-publishable", codes)
            self.assertIn("support-crate-in-non-dev-deps", codes)
            self.assertIn("support-crate-import-in-src", codes)
            self.assertIn("transitive-support-crate-dep", codes)
            # Ensure both grouped use AND qualified path expression were reported
            import_msgs = [v["message"] for v in report["violations"] if v["code"] == "support-crate-import-in-src"]
            self.assertTrue(
                any("use import" in m and "sup_alias" in m for m in import_msgs),
                import_msgs,
            )
            self.assertTrue(
                any("qualified path" in m and "usd_support" in m for m in import_msgs),
                import_msgs,
            )

        # 2) Even an EMPTY tests/support/usd_support/src/ directory is rejected
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "tests" / "support" / "usd_support" / "src").mkdir(parents=True)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("support-crate-src-dir", _codes(report))

        # 3) Support dependency cycle (`usd_support` -> `app` ->(dev) `usd_support`) is flagged
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "tests" / "support" / "usd_support" / "Cargo.toml").write_text(
                '[package]\nname = "usd_support"\nversion = "0.1.0"\nedition = "2021"\npublish = false\n\n[lib]\npath = "lib.rs"\n\n[dependencies]\napp = { path = "../../../crates/app" }\n',
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("support-architecture-cycle", _codes(report))

    def test_f04_cli_forbids_skip_cargo_metadata_and_fails_on_metadata_error(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            bad = subprocess.CompletedProcess(
                args=["cargo", "metadata"], returncode=101, stdout="", stderr="broken lockfile"
            )
            with mock.patch("check_test_architecture.subprocess.run", return_value=bad):
                report = cta.evaluate_workspace(root, skip_cargo_metadata=False)
            self.assertIn("cargo-metadata-error", _codes(report))

            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                with self.assertRaises(SystemExit) as cm:
                    cta.main(["--root", str(root), "--skip-cargo-metadata"])
            self.assertNotEqual(cm.exception.code, 0)

            stdout = io.StringIO()
            with mock.patch("check_test_architecture.subprocess.run", return_value=bad), contextlib.redirect_stdout(
                stdout
            ):
                rc = cta.main(["--root", str(root), "--json"])
            self.assertEqual(rc, 1)
            parsed = json.loads(stdout.getvalue())
            self.assertFalse(parsed["cargo_metadata_used"])

    def test_f03_fails_closed_on_filesystem_errors_and_bad_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            (root / "crates" / "app" / "src" / "bad.rs").write_bytes(b"\xff\xfe\xfd")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("utf8-decode-error", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            with mock.patch("check_test_architecture.os.scandir", side_effect=PermissionError("denied")):
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("filesystem-read-error", _codes(report))

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            outside = Path(tempfile.mkdtemp())
            try:
                _write_workspace(root)
                src_dir = root / "crates" / "app" / "src"
                real_root = Path(os.path.realpath(root))
                real_src = Path(os.path.realpath(src_dir))
                real_outside = Path(os.path.realpath(outside))
                (real_outside / "secret.rs").write_text("pub fn x() {}\n", encoding="utf-8")

                class _FakeEntry:
                    def __init__(self, path: Path, is_link: bool) -> None:
                        self.name = path.name
                        self.path = str(path)
                        self._is_link = is_link

                    def is_symlink(self) -> bool:
                        return self._is_link

                    def is_dir(self, follow_symlinks: bool = False) -> bool:
                        return False

                    def is_file(self, follow_symlinks: bool = False) -> bool:
                        return True

                fake_entries = [
                    _FakeEntry(real_src / "broken.rs", True),
                    _FakeEntry(real_src / "escape.rs", True),
                    _FakeEntry(real_src / "loop_dir", True),
                ]

                class _FakeScan:
                    def __enter__(self):
                        return iter(fake_entries)

                    def __exit__(self, *_args):
                        return False

                def _fake_realpath(p: object) -> str:
                    s = str(p)
                    if s.endswith("broken.rs"):
                        return str(real_src / "does_not_exist.rs")
                    if s.endswith("escape.rs"):
                        return str(real_outside / "secret.rs")
                    if s.endswith("loop_dir"):
                        return str(real_src)
                    return os.path.abspath(s)

                walks: list[cta.Violation] = []
                with mock.patch("check_test_architecture.os.scandir", return_value=_FakeScan()), mock.patch(
                    "check_test_architecture.os.path.realpath", side_effect=_fake_realpath
                ):
                    cta._safe_walk_tree(real_src, real_root, walks, skip_top_dirs=frozenset())

                walk_codes = {v.code for v in walks}
                self.assertIn("broken-symlink", walk_codes)
                self.assertIn("symlink-escape", walk_codes)
                self.assertIn("symlink-cycle", walk_codes)
            finally:
                for p in (outside / "secret.rs", outside):
                    if p.is_file():
                        p.unlink(missing_ok=True)
                    elif p.is_dir():
                        p.rmdir()

    def test_i4_minimum_census_is_not_a_policy_count(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["test_module_count"], 0)
            self.assertEqual(report["tests_root_count"], 0)
            self.assertIn("missing-test-modules", _codes(report))
            self.assertIn("missing-test-roots", _codes(report))
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])
            self.assertEqual(report["test_module_count"], 1)
            self.assertEqual(report["tests_root_count"], 1)
            target = root / "crates/app/tests/unit/minimum_tests.rs"
            target.write_text('// #[test] is not an implementation\n', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("missing-test-modules", _codes(report))
            self.assertEqual(report["test_module_count"], 0)
            target.write_text('#[test] fn restored() {}\n', encoding="utf-8")
            self.assertEqual(cta.evaluate_workspace(root, skip_cargo_metadata=True)["violation_count"], 0)
            target.unlink()
            (root / "crates/app/src/lib.rs").write_text("pub fn run() {}", encoding="utf-8")
            (root / "scripts/test_architecture_policy.json").write_text(json.dumps(_base_policy()), encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("missing-test-modules", _codes(report))
            self.assertIn("missing-test-roots", _codes(report))

    def test_i4_nested_external_test_module_census_and_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root)
            _add_valid_minimum(root)
            test_file = root / "crates/app/tests/unit/minimum_tests.rs"
            nested = test_file.parent / "nested.rs"
            test_file.write_text('#[path = "nested.rs"] mod nested;', encoding="utf-8")
            nested.write_text('#[test] fn nested() {}', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])
            self.assertEqual(report["test_module_count"], 1)
            test_file.write_text('// detached nested file is not reachable', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("missing-test-modules", _codes(report))
            self.assertEqual(report["test_module_count"], 0)

    def test_i4_fixture_module_identity_and_import_aliases(self) -> None:
        cases = [
            ("mod fixtures; pub fn run(){ fixtures::f(); }", True),
            ("mod fixtures; use self::fixtures as real; pub fn run(){ real::f(); }", True),
            ("mod fixtures; use self::fixtures::{f as real}; pub fn run(){ real(); }", True),
            ("mod fixtures; mod other { pub mod fixtures { pub fn f() {} } } pub fn run(){ other::fixtures::f(); }", False),
            ("mod fixtures; mod other { pub mod fixtures { pub fn f() {} } } use other::fixtures as real; pub fn run(){ real::f(); }", False),
            ("mod fixtures; use self::fixtures as real; pub fn run() {}", False),
            ("mod fixtures; use self::fixtures::f as real; pub fn run() {}", False),
            ("mod fixtures; use self::fixtures::f as real; pub fn run(real: fn()){ real(); }", False),
            ("mod fixtures; use self::fixtures::f as real; pub fn run(){ let (real,) = (|| {},); real(); }", False),
            ("mod fixtures; mod other { pub mod fixtures { pub fn f() {} } } pub fn run(){ use other::fixtures; fixtures::f(); }", False),
            ("mod fixtures; mod other { pub struct fixtures; } pub fn run(){ other::fixtures::f(); }", False),
            ("mod fixtures; pub fn run(){ let _ = stringify!(fixtures::f()); }", False),
        ]
        for source, expected in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                policy = _base_policy()
                policy["exceptional_production_fixtures"] = [{
                    "path": "crates/app/src/fixtures.rs", "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"], "reason": "real production fixture", "status": "approved",
                }]
                _write_workspace(root, policy=policy)
                (root / "crates/app/src/fixtures.rs").write_text("pub fn f() {}", encoding="utf-8")
                (root / "crates/app/src/lib.rs").write_text(source, encoding="utf-8")
                _add_valid_minimum(root)
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if expected:
                    self.assertEqual(report["violation_count"], 0, report["violations"])
                else:
                    self.assertIn("unreferenced-fixture-consumer", _codes(report))

    def test_i4_fixture_sibling_and_cross_package_identity(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["exceptional_production_fixtures"] = [{
                "path": "crates/app/src/fixtures.rs", "owner": "crates/app/src/lib.rs",
                "consumers": ["crates/app/src/consumer.rs"], "reason": "production fixture", "status": "approved",
            }]
            _write_workspace(root, policy=policy)
            src = root / "crates/app/src"
            (src / "lib.rs").write_text('mod fixtures; mod consumer;', encoding="utf-8")
            (src / "fixtures.rs").write_text('pub fn f() {}', encoding="utf-8")
            (src / "consumer.rs").write_text('use super::fixtures::{f}; pub fn run(){ f(); }', encoding="utf-8")
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])
            (src / "consumer.rs").write_text('mod other { pub mod fixtures { pub fn f() {} } } pub fn run(){ other::fixtures::f(); }', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unreferenced-fixture-consumer", _codes(report))

    def test_i4_asset_path_argument_not_any_string(self) -> None:
        cases = [
            ('const S: &[u8] = include_bytes!("shader.wgsl");', True),
            ('const S: &[u8] = include_bytes!(concat!("sha", "der.wgsl"));', True),
            ('const S: &[u8] = include_bytes!(concat!("../assets/", "shader.wgsl"));', False),
            ('use bevy::asset::load_internal_asset; fn run(){ load_internal_asset!(app, "shader.wgsl", "different.wgsl", load); }', False),
            ('use bevy::asset::{Handle, load_internal_asset}; fn run(){ load_internal_asset!(app, handle, "shader.wgsl", load); }', True),
            ('const S: &str = stringify!(include_bytes!("shader.wgsl"));', False),
            ('const S: &[u8] = other::include_bytes!("shader.wgsl");', False),
            ('use other::include_bytes; const S: &[u8] = include_bytes!("shader.wgsl");', False),
            ('macro_rules! include_bytes { ($p:expr) => { b"" }; } const S: &[u8] = include_bytes!("shader.wgsl");', False),
        ]
        for source, expected in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                policy = _base_policy()
                policy["exceptional_production_fixtures"] = [{
                    "path": "crates/app/src/shader.wgsl", "owner": "crates/app/src/lib.rs",
                    "consumers": ["crates/app/src/lib.rs"], "reason": "embedded asset", "status": "approved",
                }]
                _write_workspace(root, policy=policy)
                with (root / "crates/app/Cargo.toml").open("a", encoding="utf-8") as fh:
                    fh.write('\n[dependencies]\nbevy = "0.19"\n')
                (root / "crates/app/src/shader.wgsl").write_text("shader", encoding="utf-8")
                (root / "crates/app/src/lib.rs").write_text(source, encoding="utf-8")
                _add_valid_minimum(root)
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if expected:
                    self.assertEqual(report["violation_count"], 0, report["violations"])
                else:
                    self.assertIn("unreferenced-fixture-consumer", _codes(report))

    def test_i4_generated_producer_and_consumer_operations(self) -> None:
        good_write = 'use std::fs; use std::path::PathBuf; fn main(){ let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = root.join("../../assets/gen.txt"); fs::write(&dest, b"x").unwrap(); }'
        good_read = 'const X: &str = include_str!("../../../assets/gen.txt");'
        cases = [
            (good_write, good_read, None),
            ('fn main(){ /* assets/gen.txt */ }', good_read, "unverified-generated-producer"),
            ('fn main(){ println!("assets/gen.txt"); }', good_read, "unverified-generated-producer"),
            ('fn dormant(){ std::fs::write("../../assets/gen.txt", b"x").unwrap(); } fn main(){}', good_read, "unverified-generated-producer"),
            ('fn main(){ std::fs::write("../../other/gen.txt", b"x").unwrap(); }', good_read, "unverified-generated-producer"),
            ('fn main(){ let _ = stringify!(std::fs::write("../../assets/gen.txt", b"x")); }', good_read, "unverified-generated-producer"),
            ('mod fs { pub fn write(_: &str, _: &[u8]) {} } fn main(){ fs::write("../../assets/gen.txt", b"x"); }', good_read, "unverified-generated-producer"),
            ('fn x(){ let dest = "../../assets/gen.txt"; } fn main(){ let dest = "../../other/gen.txt"; std::fs::write(dest, b"x").unwrap(); }', good_read, "unverified-generated-producer"),
            (good_write, 'const X: &str = "assets/gen.txt";', "unverified-generated-consumer"),
            (good_write, 'const X: &str = stringify!(include_str!("../../../assets/gen.txt"));', "unverified-generated-consumer"),
            (good_write, 'const X: &str = include_str!("../../../other/gen.txt");', "unverified-generated-consumer"),
        ]
        for producer, consumer, error in cases:
            with self.subTest(producer=producer, consumer=consumer), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                policy = _base_policy()
                policy["generated_sources"] = [{"path": "assets/gen.txt", "producer": "crates/app/build.rs", "consumers": ["crates/app/src/lib.rs"], "reason": "generated asset", "status": "approved"}]
                _write_workspace(root, policy=policy)
                (root / "assets").mkdir()
                (root / "assets/gen.txt").write_text("x", encoding="utf-8")
                (root / "crates/app/build.rs").write_text(producer, encoding="utf-8")
                (root / "crates/app/src/lib.rs").write_text(consumer, encoding="utf-8")
                _add_valid_minimum(root)
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if error:
                    self.assertIn(error, _codes(report))
                else:
                    self.assertEqual(report["violation_count"], 0, report["violations"])

    def test_i4_repository_asset_contract_with_root_producer(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["generated_sources"] = [{
                "path": "assets/external/usdz_sample.usdz", "producer": "build.rs",
                "consumers": ["crates/app/tests/unit/minimum_tests.rs"],
                "reason": "repository build asset", "status": "approved",
            }]
            _write_workspace(root, members=["crates/app", "tests/support/usd"], policy=policy)
            (root / "tests/support/usd_support").rename(root / "tests/support/usd")
            (root / "tests/support/usd/lib.rs").write_text('''
use std::path::{Component, Path, PathBuf};
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().and_then(Path::parent).and_then(Path::parent)
        .expect("usd_support crate is beneath tests/support/usd")
}
fn validate_relative_path<'a>(relative: &'a str, domain: &str) -> &'a Path {
    let path = Path::new(relative);
    assert!(
        !path.as_os_str().is_empty()
            && path.components().all(|part| matches!(part, Component::Normal(_))),
        "{domain} must be a relative path with normal components only: {}",
        path.display()
    );
    path
}
pub fn resolve_asset_path(relative: &str) -> PathBuf {
    let relative = validate_relative_path(relative, "asset");
    let path = workspace_root().join("assets").join(relative);
    assert!(path.is_file(), "asset file does not exist: {}", path.display());
    path
}
''', encoding="utf-8")
            app_manifest = root / "crates/app/Cargo.toml"
            app_manifest.write_text(app_manifest.read_text().replace("tests/support/usd_support", "tests/support/usd"), encoding="utf-8")
            root_manifest = root / "Cargo.toml"
            root_manifest.write_text('[package]\nname="usdview"\nversion="0.1.0"\n' + root_manifest.read_text(), encoding="utf-8")
            (root / "src").mkdir()
            (root / "src/lib.rs").write_text("pub fn root() {}", encoding="utf-8")
            (root / "assets/external").mkdir(parents=True)
            (root / "assets/external/usdz_sample.usdz").write_bytes(b"asset")
            (root / "build.rs").write_text('use std::fs; use std::path::PathBuf; fn main(){ let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = workspace_root.join("assets/external/usdz_sample.usdz"); let bytes = b"asset"; if let Err(e) = fs::write(&dest, bytes) { eprintln!("{e}"); } }', encoding="utf-8")
            _add_valid_minimum(root)
            test = root / "crates/app/tests/unit/minimum_tests.rs"
            test.write_text('#[test] fn t(){ let archive = usd_support::resolve_asset_path("external/usdz_sample.usdz").canonicalize().unwrap(); let _ = archive; }', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])
            app_manifest.write_text(app_manifest.read_text().replace('usd_support = { path = "../../tests/support/usd" }', ''), encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unverified-generated-consumer", _codes(report))

    def test_i4_generated_src_asset_cannot_be_test_only(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["generated_sources"] = [{
                "path": "crates/app/src/sample.txt", "producer": "crates/app/build.rs",
                "consumers": ["crates/app/tests/unit/minimum_tests.rs"], "reason": "generated fixture", "status": "approved",
            }]
            _write_workspace(root, policy=policy)
            _add_valid_minimum(root)
            (root / "crates/app/src/sample.txt").write_text("sample", encoding="utf-8")
            (root / "crates/app/build.rs").write_text('fn main(){ std::fs::write("src/sample.txt", b"sample").unwrap(); }', encoding="utf-8")
            (root / "crates/app/tests/unit/minimum_tests.rs").write_text('#[test] fn t(){ let _ = include_str!("../../src/sample.txt"); }', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("test-only-generated-fixture-in-src", _codes(report))
            self.assertIn("unclassified-production-fixture", _codes(report))

    def test_i4_out_dir_single_exact_package_identity(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["generated_sources"] = [{
                "path": "OUT_DIR:crates/app/nested/gen.rs", "producer": "crates/app/build.rs",
                "consumers": ["crates/app/src/lib.rs"], "reason": "generated Rust", "status": "approved",
            }]
            _write_workspace(root, policy=policy)
            (root / "crates/app/build.rs").write_text('use std::path::PathBuf; fn main(){ let p = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("nested/gen.rs"); std::fs::write(p, b"const X: u8 = 0;").unwrap(); }', encoding="utf-8")
            src = root / "crates/app/src/lib.rs"
            src.write_text('include!(concat!(env!("OUT_DIR"), "/nested/gen.rs"));', encoding="utf-8")
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])
            original = src.read_text(encoding="utf-8")
            for wrong in ('/different/gen.rs', '/../nested/gen.rs'):
                src.write_text(original.replace('/nested/gen.rs', wrong), encoding="utf-8")
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                self.assertIn("unclassified-generated-source", _codes(report))
            src.write_text(original, encoding="utf-8")
            self.assertEqual(cta.evaluate_workspace(root, skip_cargo_metadata=True)["violation_count"], 0)

    def test_i4_support_to_support_and_nontrivial_cycle(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_workspace(root, members=["crates/app", "crates/b", "tests/support/usd_support", "tests/support/z_support"])
            b = root / "crates/b"
            (b / "src").mkdir(parents=True)
            (b / "src/lib.rs").write_text("pub fn b() {}", encoding="utf-8")
            (b / "Cargo.toml").write_text('[package]\nname="b"\nversion="0.1.0"\n[dev-dependencies]\napp={path="../app"}\n', encoding="utf-8")
            z = root / "tests/support/z_support"
            z.mkdir(parents=True)
            (z / "lib.rs").write_text("pub fn z() {}", encoding="utf-8")
            (z / "Cargo.toml").write_text('[package]\nname="z_support"\nversion="0.1.0"\npublish=false\n[lib]\npath="lib.rs"\n[dev-dependencies]\nb={path="../../../crates/b"}\n', encoding="utf-8")
            app = root / "crates/app/Cargo.toml"
            with app.open("a", encoding="utf-8") as fh:
                fh.write('b={path="../b"}\nz_support={path="../../tests/support/z_support"}\n')
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("support-architecture-cycle", _codes(report))
            # Remove the cycle, then introduce acyclic normal support-to-support leakage.
            (z / "Cargo.toml").write_text('[package]\nname="z_support"\nversion="0.1.0"\npublish=false\n[lib]\npath="lib.rs"\n[dependencies]\nusd_support={path="../usd_support"}\n', encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("support-crate-in-non-dev-deps", _codes(report))
            self.assertNotIn("support-architecture-cycle", _codes(report))
            (z / "Cargo.toml").write_text((z / "Cargo.toml").read_text().replace('[dependencies]', '[dev-dependencies]'), encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

    def test_i4_parent_import_submodule_consumer_and_visibility(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["exceptional_production_fixtures"] = [{
                "path": "crates/app/src/fixtures.rs",
                "owner": "crates/app/src/lib.rs",
                "consumers": ["crates/app/src/child_impl.rs"],
                "reason": "production fixture used by path-attached submodule via parent import",
                "status": "approved",
            }]
            _write_workspace(root, policy=policy)
            src = root / "crates/app/src"
            (src / "fixtures.rs").write_text("pub fn f() {}", encoding="utf-8")
            (src / "parent.rs").write_text(
                'use super::fixtures;\n#[path = "child_impl.rs"]\nmod child;\n',
                encoding="utf-8",
            )
            (src / "child_impl.rs").write_text(
                "use super::fixtures;\npub fn run() { fixtures::f(); }\n",
                encoding="utf-8",
            )
            (src / "lib.rs").write_text("mod fixtures;\nmod parent;\n", encoding="utf-8")
            _add_valid_minimum(root)
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertEqual(report["violation_count"], 0, report["violations"])

            # Unused import in child_impl fails
            (src / "child_impl.rs").write_text("use super::fixtures;\npub fn run() {}\n", encoding="utf-8")
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unreferenced-fixture-consumer", _codes(report))

            # Parent re-exporting fake shadow module fails
            (src / "parent.rs").write_text(
                'mod other { pub mod fixtures { pub fn f() {} } }\nuse other::fixtures;\n#[path = "child_impl.rs"]\nmod child;\n',
                encoding="utf-8",
            )
            (src / "child_impl.rs").write_text(
                "use super::fixtures;\npub fn run() { fixtures::f(); }\n",
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unreferenced-fixture-consumer", _codes(report))

            # Non-descendant cannot exploit parent's private `use super::fixtures;`
            (src / "parent.rs").write_text("use super::fixtures;\n", encoding="utf-8")
            (src / "child_impl.rs").write_text("pub fn run() { crate::parent::fixtures::f(); }\n", encoding="utf-8")
            (src / "lib.rs").write_text(
                "mod fixtures;\nmod parent;\nmod child_impl;\n",
                encoding="utf-8",
            )
            report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
            self.assertIn("unreferenced-fixture-consumer", _codes(report))


    def test_i5_actual_attachment_context_and_owner_reachability(self) -> None:
        for mode in ("direct", "macro", "macro_invocation", "macro_parens", "orphan", "reachable", "nested_macro", "nested_actual"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                _write_workspace(root)
                _add_valid_minimum(root)
                src = root / "crates/app/src/lib.rs"
                target = root / "crates/app/tests/unit/minimum_tests.rs"
                policy_path = root / "scripts/test_architecture_policy.json"
                attachment = '#[cfg(test)]\n#[path = "../tests/unit/minimum_tests.rs"]\nmod minimum_tests;\n'
                if mode == "macro":
                    src.write_text('macro_rules! unused_attachment { () => { ' + attachment + ' }; }\npub fn run() {}', encoding="utf-8")
                elif mode in {"macro_invocation", "macro_parens"}:
                    opening, closing = ("{", "}") if mode == "macro_invocation" else ("(", ");")
                    src.write_text('macro_rules! discard { ($($t:tt)*) => {} }\ndiscard! ' + opening + attachment + closing, encoding="utf-8")
                elif mode in {"orphan", "reachable"}:
                    (src.parent / "orphan.rs").write_text(attachment, encoding="utf-8")
                    src.write_text('mod orphan;' if mode == "reachable" else 'pub fn run() {}', encoding="utf-8")
                    pol = json.loads(policy_path.read_text())
                    pol["reviewed_path_only_attachments"][0]["owner_file"] = "crates/app/src/orphan.rs"
                    policy_path.write_text(json.dumps(pol), encoding="utf-8")
                elif mode in {"nested_macro", "nested_actual"}:
                    (target.parent / "child.rs").write_text('#[test] fn t() {}', encoding="utf-8")
                    child = '#[path = "child.rs"] mod child;'
                    target.write_text('macro_rules! dormant { () => { ' + child + ' }; }' if mode == "nested_macro" else child, encoding="utf-8")
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if mode in {"direct", "reachable", "nested_actual"}:
                    self.assertEqual(report["violation_count"], 0, report["violations"])
                    self.assertEqual(report["test_module_count"], 1)
                    self.assertEqual(report["tests_root_count"], 1)
                else:
                    self.assertIn("missing-test-modules", _codes(report))
                    self.assertIn("missing-test-roots", _codes(report))
                    self.assertEqual(report["test_module_count"], 0)
                    self.assertEqual(report["tests_root_count"], 0)
                    if mode in {"macro", "macro_invocation", "macro_parens"}:
                        self.assertIn("non-direct-test-module-attachment", _codes(report))
                    if mode == "orphan":
                        self.assertIn("unreachable-reviewed-attachment", _codes(report))

    def test_i5_asset_macro_identity_and_lexical_imports(self) -> None:
        fake = '#[macro_export] macro_rules! fake_asset { ($a:expr,$b:expr,$c:expr,$d:expr) => { () }; } '
        cases = [
            (fake + 'mod bevy { pub mod asset { pub use crate::fake_asset as load_internal_asset; } } fn run(){ bevy::asset::load_internal_asset!((),(),"shader.wgsl",()); }', False),
            (fake + 'mod sibling { use bevy::asset::load_internal_asset; } fn run(){ load_internal_asset!((),(),"shader.wgsl",()); }', False),
            ('fn run(){ { use bevy::asset::load_internal_asset; } load_internal_asset!((),(),"shader.wgsl",()); }', False),
            ('use bevy::asset::load_internal_asset as load; fn run(){ load!((),(),"shader.wgsl",()); }', True),
            (fake + 'mod local { pub mod asset { pub use crate::fake_asset as load_internal_asset; } } use crate::local as bevy; use bevy::asset::load_internal_asset; fn run(){ load_internal_asset!((),(),"shader.wgsl",()); }', False),
            ('macro_rules! concat { ($($t:tt)*) => { "other.wgsl" }; } mod nested { const X: &[u8] = include_bytes!(concat!("shader", ".wgsl")); }', False),
            ('fn run(){ ::bevy::asset::load_internal_asset!((),(),"shader.wgsl",()); }', True),
            ('macro_rules! concat { ($($t:tt)*) => { "other.wgsl" }; } const X: &[u8] = include_bytes!(concat!("shader", ".wgsl"));', False),
            ('const X: &[u8] = include_bytes!(concat!("shader", ".wgsl"));', True),
            ('#[macro_export] macro_rules! fake_inc { ($a:expr) => { b"" }; } mod std { pub use crate::fake_inc as include_bytes; } const X: &[u8] = std::include_bytes!("shader.wgsl");', False),
        ]
        for source, expected in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                policy = _base_policy()
                policy["exceptional_production_fixtures"] = [{"path":"crates/app/src/shader.wgsl", "owner":"crates/app/src/lib.rs", "consumers":["crates/app/src/lib.rs"], "reason":"embedded asset", "status":"approved"}]
                _write_workspace(root, policy=policy)
                with (root / "crates/app/Cargo.toml").open("a", encoding="utf-8") as fh:
                    fh.write('\n[dependencies]\nbevy = "0.19"\n')
                (root / "crates/app/src/shader.wgsl").write_text("shader", encoding="utf-8")
                (root / "crates/app/src/lib.rs").write_text(source, encoding="utf-8")
                _add_valid_minimum(root)
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if expected:
                    self.assertEqual(report["violation_count"], 0, report["violations"])
                else:
                    self.assertIn("unreferenced-fixture-consumer", _codes(report))

    def test_i5_generation_execution_owner_and_path_identity(self) -> None:
        cases = [
            ('fn main(){ fn dormant(){ std::fs::write("assets/gen.txt", b"x").unwrap(); } }', False),
            ('fn main(){ let dormant = || { std::fs::write("assets/gen.txt", b"x").unwrap(); }; }', False),
            ('fn main(){ let dormant = || std::fs::write("assets/gen.txt", b"x").unwrap(); }', False),
            ('fn main(){ let dormant = async { std::fs::write("assets/gen.txt", b"x").unwrap(); }; }', False),
            ('fn main(){ if let Err(e) = std::fs::write("assets/gen.txt", b"x") { eprintln!("{e}"); } }', True),
            ('struct PathBuf; impl PathBuf { fn from(p: &str) -> std::path::PathBuf { std::path::PathBuf::from(p).join("other") } } fn main(){ let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = root.join("assets/gen.txt"); std::fs::write(&dest,b"x").unwrap(); }', False),
            ('use std::path::PathBuf; fn main(){ let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = root.join("assets/gen.txt"); std::fs::write(&dest,b"x").unwrap(); }', True),
            ('fn main(){ let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = root.join("assets/gen.txt"); std::fs::write(&dest,b"x").unwrap(); }', True),
            ('use std::path::PathBuf; macro_rules! env { ($p:expr) => { "wrong" }; } fn main(){ let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")); let dest = root.join("assets/gen.txt"); std::fs::write(&dest,b"x").unwrap(); }', False),
            ('fn main(){ let dest = "assets/gen.txt"; let dest: &str = "other/gen.txt"; std::fs::write(dest,b"x").unwrap(); }', False),
            ('fn main(){ let dest = "assets/gen.txt"; for dest in ["other/gen.txt"] { std::fs::write(dest,b"x").unwrap(); } }', False),
            ('trait Join { fn join(&self, _: &str) -> std::path::PathBuf; } impl Join for str { fn join(&self, _: &str) -> std::path::PathBuf { std::path::PathBuf::from("other/gen.txt") } } fn main(){ let root = "assets"; let dest = root.join("gen.txt"); std::fs::write(dest,b"x").unwrap(); }', False),
        ]
        for producer, expected in cases:
            with self.subTest(producer=producer), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                policy = _base_policy()
                policy["generated_sources"] = [{"path":"crates/app/assets/gen.txt", "producer":"crates/app/build.rs", "consumers":["crates/app/src/lib.rs"], "reason":"generated asset", "status":"approved"}]
                _write_workspace(root, policy=policy)
                app = root / "crates/app"
                (app / "assets").mkdir()
                (app / "assets/gen.txt").write_text("x", encoding="utf-8")
                (app / "build.rs").write_text(producer, encoding="utf-8")
                (app / "src/lib.rs").write_text('const X: &str = include_str!("../assets/gen.txt");', encoding="utf-8")
                _add_valid_minimum(root)
                report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                if expected:
                    self.assertEqual(report["violation_count"], 0, report["violations"])
                else:
                    self.assertIn("unverified-generated-producer", _codes(report))


    def test_i6_generated_filesystem_operation_identity(self) -> None:
        """An imported name is not proof of its filesystem operation or argument role."""
        direct_write = 'fn main(){ std::fs::write("assets/gen.txt", b"x").unwrap(); }'
        include_read = 'const X: &str = include_str!("../assets/gen.txt");'
        copy_as_write = 'use std::fs::copy as write; fn main(){ write("assets/gen.txt", "assets/other.txt").unwrap(); }'
        write_as_save = 'use std::fs::write as save; fn main(){ save("assets/gen.txt", b"x").unwrap(); }'
        remove_as_read = 'use std::fs::remove_file as read; pub fn run(){ read("assets/gen.txt").unwrap(); }'
        cases = [
            ("direct", direct_write, include_read, None),
            ("copy source is not write destination", copy_as_write, include_read, "unverified-generated-producer"),
            ("actual write alias", write_as_save, include_read, None),
            ("copy alias repeated", copy_as_write, include_read, "unverified-generated-producer"),
            ("rename source is not write destination", 'use std::fs::rename as write; fn main(){ write("assets/gen.txt", "assets/other.txt").unwrap(); }', include_read, "unverified-generated-producer"),
            ("grouped write alias", 'use std::fs::{write as save}; fn main(){ save("assets/gen.txt", b"x").unwrap(); }', include_read, None),
            ("write alias wrong destination", 'use std::fs::write as save; fn main(){ save("assets/other.txt", b"x").unwrap(); }', include_read, "unverified-generated-producer"),
            ("write alias wrong arity", 'use std::fs::write as save; fn main(){ save("assets/gen.txt"); }', include_read, "unverified-generated-producer"),
            ("write alias dormant helper", 'use std::fs::write as save; fn main(){ fn dormant(){ save("assets/gen.txt", b"x").unwrap(); } }', include_read, "unverified-generated-producer"),
            ("direct read", direct_write, 'pub fn run(){ let _ = std::fs::read("assets/gen.txt").unwrap(); }', None),
            ("remove is not read", direct_write, remove_as_read, "unverified-generated-consumer"),
            ("actual read alias", direct_write, 'use std::fs::read as load; pub fn run(){ let _ = load("assets/gen.txt").unwrap(); }', None),
            ("remove alias repeated", direct_write, remove_as_read, "unverified-generated-consumer"),
            ("metadata is not read_to_string", direct_write, 'use std::fs::metadata as read_to_string; pub fn run(){ let _ = read_to_string("assets/gen.txt").unwrap(); }', "unverified-generated-consumer"),
            ("actual string reader alias", direct_write, 'use std::fs::read_to_string as load_text; pub fn run(){ let _ = load_text("assets/gen.txt").unwrap(); }', None),
            ("reader wrong source", direct_write, 'use std::fs::read as load; pub fn run(){ let _ = load("assets/other.txt").unwrap(); }', "unverified-generated-consumer"),
            ("reader wrong arity", direct_write, 'use std::fs::read as load; pub fn run(){ let _ = load("assets/gen.txt", "other"); }', "unverified-generated-consumer"),
            ("reader shadowed by local function", direct_write, 'fn read(_: &str) {} pub fn run(){ read("assets/gen.txt"); }', "unverified-generated-consumer"),
        ]
        # Reuse the same fixture to test fail -> pass -> fail without changing
        # classification or test wiring. The Rust strings are never executed.
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            policy = _base_policy()
            policy["generated_sources"] = [{
                "path": "crates/app/assets/gen.txt",
                "producer": "crates/app/build.rs",
                "consumers": ["crates/app/src/lib.rs"],
                "reason": "generated asset",
                "status": "approved",
            }]
            _write_workspace(root, policy=policy)
            app = root / "crates/app"
            (app / "assets").mkdir()
            (app / "assets/gen.txt").write_text("x", encoding="utf-8")
            src = app / "src/lib.rs"
            _add_valid_minimum(root)
            attachment = src.read_text(encoding="utf-8")
            for label, producer, consumer, error in cases:
                with self.subTest(case=label):
                    (app / "build.rs").write_text(producer, encoding="utf-8")
                    src.write_text(consumer + "\n" + attachment, encoding="utf-8")
                    report = cta.evaluate_workspace(root, skip_cargo_metadata=True)
                    self.assertEqual(report["test_module_count"], 1)
                    self.assertEqual(report["tests_root_count"], 1)
                    if error is None:
                        self.assertEqual(report["status"], "pass", report["violations"])
                        self.assertEqual(report["violation_count"], 0, report["violations"])
                    else:
                        self.assertEqual(report["status"], "fail")
                        self.assertEqual(_codes(report), {error}, report["violations"])
                        self.assertEqual(report["violation_count"], 1, report["violations"])


if __name__ == "__main__":
    unittest.main()
