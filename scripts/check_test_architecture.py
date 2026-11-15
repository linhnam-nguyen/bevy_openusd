#!/usr/bin/env python3
"""Canonical backend test-architecture and fixture-classification evaluator.

Enforces BO-M9-C1 across every Rust workspace member in bevy_openusd:
- Zero non-path #[cfg(test)], #[cfg(not(test))], cfg!(test), or test attributes under any src/ root
- Strictly ONE direct #[cfg(test)] + ONE direct unconditional #[path = "..."] + zero non-lint attrs
  for every external test module attachment, matched 1:1 against reviewed_path_only_attachments
- Module-context-relative #[path = "..."] resolution strictly outside ALL src/ roots and target/
- Rejects empty or populated src/**/tests/ directories and any support-crate src/ directory
- Verifies dynamic tests/support/* crates (publish=false, explicit [lib] path="lib.rs", acyclic
  architecture, zero normal/build/transitive/source leakage in non-support src/)
- Classifies generated_sources and exceptional_production_fixtures via strict AST/token producer
  and consumer provenance checks (no string-substring or shadowed-local-variable bypasses)
- Authoritative cargo metadata required on CLI (--no-deps declared workspace graph closure;
  synthetic metadata-less fallback restricted to internal evaluate_workspace() test helper)
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import fnmatch
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib
from typing import Any


REQUIRED_POLICY_KEYS = frozenset(
    {
        "test_support_crates",
        "forbidden_test_file_names",
        "forbidden_test_attr_suffixes",
        "reviewed_path_only_attachments",
        "generated_sources",
        "exceptional_production_fixtures",
    }
)
FORBIDDEN_POLICY_KEYS = frozenset({"reviewed_seams"})
MANDATORY_FORBIDDEN_TEST_FILE_NAMES = frozenset(
    {
        "tests.rs",
        "test.rs",
        "test_support.rs",
        "test_helpers.rs",
        "test_fixtures.rs",
    }
)
MANDATORY_FORBIDDEN_TEST_ATTR_SUFFIXES = frozenset({"test", "rstest", "test_case"})
FORBIDDEN_INLINE_MOD_EXACT = frozenset({"tests", "test", "test_support", "test_helpers", "test_fixtures"})
ALLOWED_TEST_MOD_EXTRA_ATTRS = frozenset({"allow", "expect", "warn", "deny", "forbid", "doc"})
ASSET_LOAD_MACROS = frozenset(
    {"load_internal_asset", "include_str", "include_bytes", "embedded_asset", "embedded_path"}
)
CANDIDATE_FIXTURE_KEYWORDS = (
    "fixture",
    "sample",
    "mock",
    "stub",
    "fake",
    "dummy",
    "harness",
    "support",
    "helper",
    "test",
)
VALID_APPROVAL_STATUSES = frozenset({"approved"})
SKIP_WALK_TOP_DIRS = frozenset({".git", "target"})


@dataclass(frozen=True)
class Violation:
    code: str
    path: str
    line: int
    message: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "code": self.code,
            "path": self.path,
            "line": self.line,
            "message": self.message,
        }


@dataclass(frozen=True)
class Token:
    kind: str  # IDENT, STRING, CHAR, LIFETIME, PUNCT, LEX_ERROR
    value: str
    raw: str
    line: int
    col: int


@dataclass
class AttrNode:
    kind: str  # "path" | "call" | "assign" | "other"
    segments: tuple[str, ...] = ()
    args: list["AttrNode"] = field(default_factory=list)
    value: Token | None = None
    line: int = 1


@dataclass(frozen=True)
class AttachmentRecord:
    owner_file: str
    module_path: str
    mod_name: str
    target: str
    line: int

    def identity(self) -> tuple[str, str, str, str]:
        return (self.owner_file, self.module_path, self.mod_name, self.target)


@dataclass(frozen=True)
class ModBinding:
    owner_rel: str
    local_name: str
    resolved_target: Path
    line: int
    scope: tuple[str, ...] = ()


@dataclass(frozen=True)
class IncludeMarker:
    origin_rel: str
    line: int
    kind: str  # "include_literal" | "include_out_dir" | "out_dir" | "include_dynamic"
    raw_value: str | None
    resolved_target: Path | None


@dataclass
class PackageInfo:
    name: str
    norm_name: str
    pkg_id: str
    dir_path: Path
    rel_dir: str
    manifest_path: Path
    is_support: bool
    publish_disabled: bool
    src_roots: list[Path]
    prod_target_files: list[Path]
    metadata_deps: list[dict[str, Any]] = field(default_factory=list)
    metadata_targets: list[dict[str, Any]] = field(default_factory=list)


def _to_posix_rel(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def _norm_crate_name(name: str) -> str:
    return name.replace("-", "_")


def _norm_ident(raw: str) -> str:
    return raw[2:] if raw.startswith("r#") else raw


def _is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def _read_text_strict(path: Path, rel_path: str, violations: list[Violation]) -> str | None:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        violations.append(Violation("filesystem-read-error", rel_path, 1, f"cannot read file: {exc}"))
        return None
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        violations.append(Violation("utf8-decode-error", rel_path, 1, f"invalid UTF-8: {exc}"))
        return None


def _validate_policy(policy_path: Path, root_real: Path, violations: list[Violation]) -> dict[str, Any] | None:
    rel = _to_posix_rel(policy_path, root_real) if _is_within(policy_path, root_real) else policy_path.name
    if not policy_path.exists():
        violations.append(Violation("missing-policy-file", rel, 1, "test architecture policy JSON not found"))
        return None
    text = _read_text_strict(policy_path, rel, violations)
    if text is None:
        return None
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        violations.append(Violation("invalid-policy-json", rel, exc.lineno, f"invalid JSON: {exc.msg}"))
        return None
    if not isinstance(data, dict):
        violations.append(Violation("invalid-policy-schema", rel, 1, "policy root must be a JSON object"))
        return None

    keys = set(data.keys())
    forbidden_present = sorted(keys & FORBIDDEN_POLICY_KEYS)
    if forbidden_present:
        violations.append(
            Violation(
                "forbidden-policy-key",
                rel,
                1,
                f"policy contains removed exemption key(s): {', '.join(forbidden_present)}",
            )
        )
    missing = sorted(REQUIRED_POLICY_KEYS - keys)
    extra = sorted((keys - REQUIRED_POLICY_KEYS) - FORBIDDEN_POLICY_KEYS)
    if missing:
        violations.append(
            Violation("invalid-policy-schema", rel, 1, f"missing policy key(s): {', '.join(missing)}")
        )
    if extra:
        violations.append(
            Violation("invalid-policy-schema", rel, 1, f"unexpected policy key(s): {', '.join(extra)}")
        )
    if forbidden_present or missing or extra:
        return None

    for list_key in ("test_support_crates", "forbidden_test_file_names", "forbidden_test_attr_suffixes"):
        val = data[list_key]
        if not isinstance(val, list) or not all(isinstance(x, str) and x.strip() for x in val):
            violations.append(
                Violation("invalid-policy-schema", rel, 1, f"{list_key} must be a list of non-empty strings")
            )
            return None

    if not data["test_support_crates"]:
        violations.append(
            Violation("policy-weakened-minimums", rel, 1, "test_support_crates must not be empty")
        )
        return None

    missing_names = sorted(MANDATORY_FORBIDDEN_TEST_FILE_NAMES - set(data["forbidden_test_file_names"]))
    if missing_names:
        violations.append(
            Violation(
                "policy-weakened-minimums",
                rel,
                1,
                f"forbidden_test_file_names is missing mandatory entries: {', '.join(missing_names)}",
            )
        )
        return None

    missing_suffixes = sorted(MANDATORY_FORBIDDEN_TEST_ATTR_SUFFIXES - set(data["forbidden_test_attr_suffixes"]))
    if missing_suffixes:
        violations.append(
            Violation(
                "policy-weakened-minimums",
                rel,
                1,
                f"forbidden_test_attr_suffixes is missing mandatory entries: {', '.join(missing_suffixes)}",
            )
        )
        return None

    for key in ("reviewed_path_only_attachments", "generated_sources", "exceptional_production_fixtures"):
        if not isinstance(data[key], list):
            violations.append(Violation("invalid-policy-schema", rel, 1, f"{key} must be an array"))
            return None

    for idx, entry in enumerate(data["reviewed_path_only_attachments"]):
        req = {"owner_file", "module_path", "mod_name", "target"}
        if not isinstance(entry, dict) or set(entry.keys()) != req:
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"reviewed_path_only_attachments[{idx}] must have exact keys {sorted(req)}",
                )
            )
            return None
        for k in ("owner_file", "mod_name", "target"):
            if not isinstance(entry[k], str) or not entry[k].strip():
                violations.append(
                    Violation(
                        "invalid-policy-schema",
                        rel,
                        1,
                        f"reviewed_path_only_attachments[{idx}].{k} must be a non-empty string",
                    )
                )
                return None
        if not isinstance(entry["module_path"], str):
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"reviewed_path_only_attachments[{idx}].module_path must be a string",
                )
            )
            return None

    for idx, entry in enumerate(data["generated_sources"]):
        req = {"path", "producer", "consumers", "reason", "status"}
        if not isinstance(entry, dict) or set(entry.keys()) != req:
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"generated_sources[{idx}] must have exact keys {sorted(req)}",
                )
            )
            return None
        for k in ("path", "producer", "reason"):
            if not isinstance(entry[k], str) or not entry[k].strip():
                violations.append(
                    Violation(
                        "invalid-policy-schema",
                        rel,
                        1,
                        f"generated_sources[{idx}].{k} must be a non-empty string",
                    )
                )
                return None
        if entry["status"] not in VALID_APPROVAL_STATUSES:
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"generated_sources[{idx}].status must be one of {sorted(VALID_APPROVAL_STATUSES)}",
                )
            )
            return None
        consumers = entry["consumers"]
        if not isinstance(consumers, list) or not consumers or not all(
            isinstance(c, str) and c.strip() for c in consumers
        ):
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"generated_sources[{idx}].consumers must be a non-empty list of strings",
                )
            )
            return None

    for idx, entry in enumerate(data["exceptional_production_fixtures"]):
        req = {"path", "owner", "consumers", "reason", "status"}
        if not isinstance(entry, dict) or set(entry.keys()) != req:
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"exceptional_production_fixtures[{idx}] must have exact keys {sorted(req)}",
                )
            )
            return None
        for k in ("path", "owner", "reason"):
            if not isinstance(entry[k], str) or not entry[k].strip():
                violations.append(
                    Violation(
                        "invalid-policy-schema",
                        rel,
                        1,
                        f"exceptional_production_fixtures[{idx}].{k} must be a non-empty string",
                    )
                )
                return None
        if entry["status"] not in VALID_APPROVAL_STATUSES:
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"exceptional_production_fixtures[{idx}].status must be one of {sorted(VALID_APPROVAL_STATUSES)}",
                )
            )
            return None
        consumers = entry["consumers"]
        if not isinstance(consumers, list) or not consumers or not all(
            isinstance(c, str) and c.strip() for c in consumers
        ):
            violations.append(
                Violation(
                    "invalid-policy-schema",
                    rel,
                    1,
                    f"exceptional_production_fixtures[{idx}].consumers must be a non-empty list of strings",
                )
            )
            return None

    return data


def _safe_walk_tree(
    start_dir: Path,
    root_real: Path,
    violations: list[Violation],
    *,
    skip_top_dirs: frozenset[str] = SKIP_WALK_TOP_DIRS,
) -> tuple[list[Path], list[Path]]:
    """Safely enumerate (directories, files) under start_dir, failing closed on errors & bad symlinks."""
    discovered_dirs: list[Path] = []
    discovered_files: list[Path] = []
    start_real = Path(os.path.realpath(start_dir))
    if not _is_within(start_real, root_real):
        violations.append(
            Violation("symlink-escape", str(start_dir), 1, f"walk root escapes workspace: {start_real}")
        )
        return discovered_dirs, discovered_files

    discovered_dirs.append(start_dir)

    def _recurse(curr_dir: Path, active_ancestors: set[str]) -> None:
        try:
            with os.scandir(curr_dir) as it:
                entries = sorted(it, key=lambda e: e.name)
        except OSError as exc:
            rel = _to_posix_rel(curr_dir, root_real) if _is_within(curr_dir, root_real) else str(curr_dir)
            violations.append(Violation("filesystem-read-error", rel, 1, f"cannot scan directory: {exc}"))
            return

        for entry in entries:
            entry_path = Path(entry.path)
            rel_entry = _to_posix_rel(entry_path, root_real) if _is_within(entry_path, root_real) else entry.name
            if curr_dir == start_dir and entry.name in skip_top_dirs:
                continue
            if entry.name == "target" and entry_path.parent == root_real:
                continue

            try:
                is_link = entry.is_symlink()
            except OSError as exc:
                violations.append(Violation("filesystem-read-error", rel_entry, 1, f"stat failed: {exc}"))
                continue

            if is_link:
                try:
                    link_target = Path(os.path.realpath(entry_path))
                except OSError as exc:
                    violations.append(
                        Violation("filesystem-read-error", rel_entry, 1, f"cannot resolve symlink: {exc}")
                    )
                    continue
                if not link_target.exists():
                    violations.append(
                        Violation("broken-symlink", rel_entry, 1, "broken symlink inside scanned tree")
                    )
                    continue
                if not _is_within(link_target, root_real):
                    violations.append(
                        Violation(
                            "symlink-escape",
                            rel_entry,
                            1,
                            f"symlink escapes workspace root to {link_target}",
                        )
                    )
                    continue
                if link_target.is_dir():
                    discovered_dirs.append(entry_path)
                    real_str = str(link_target)
                    if real_str in active_ancestors:
                        violations.append(
                            Violation("symlink-cycle", rel_entry, 1, "symlink directory cycle detected")
                        )
                        continue
                    _recurse(entry_path, active_ancestors | {real_str})
                elif link_target.is_file():
                    discovered_files.append(entry_path)
                continue

            try:
                if entry.is_dir(follow_symlinks=False):
                    discovered_dirs.append(entry_path)
                    real_str = str(Path(os.path.realpath(entry_path)))
                    _recurse(entry_path, active_ancestors | {real_str})
                elif entry.is_file(follow_symlinks=False):
                    discovered_files.append(entry_path)
            except OSError as exc:
                violations.append(Violation("filesystem-read-error", rel_entry, 1, f"entry inspect failed: {exc}"))

    _recurse(start_dir, {str(start_real)})
    return discovered_dirs, discovered_files


def _load_toml(path: Path, root_real: Path, violations: list[Violation]) -> dict[str, Any] | None:
    rel = _to_posix_rel(path, root_real) if _is_within(path, root_real) else path.name
    text = _read_text_strict(path, rel, violations)
    if text is None:
        return None
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        violations.append(Violation("cargo-toml-error", rel, 1, f"TOML parse error: {exc}"))
        return None


def _expand_workspace_globs(root_real: Path, patterns: list[str], excludes: list[str]) -> set[Path]:
    matched: set[Path] = set()
    for pat in patterns:
        if pat == ".":
            matched.add(root_real)
            continue
        if any(ch in pat for ch in "*?["):
            for candidate in root_real.glob(pat):
                if candidate.is_dir() and (candidate / "Cargo.toml").is_file():
                    matched.add(Path(os.path.realpath(candidate)))
        else:
            candidate = root_real / pat
            if candidate.is_dir() and (candidate / "Cargo.toml").is_file():
                matched.add(Path(os.path.realpath(candidate)))

    excluded_paths: set[Path] = set()
    for ex in excludes:
        for cand in list(matched):
            rel = cand.relative_to(root_real).as_posix()
            if fnmatch.fnmatch(rel, ex) or rel == ex:
                excluded_paths.add(cand)
    return matched - excluded_paths


def _discover_workspace(
    root_real: Path,
    policy: dict[str, Any],
    skip_cargo_metadata: bool,
    violations: list[Violation],
) -> tuple[list[PackageInfo], dict[str, str], bool]:
    root_cargo = root_real / "Cargo.toml"
    root_toml = _load_toml(root_cargo, root_real, violations)
    if root_toml is None:
        return [], {}, False

    ws_table = root_toml.get("workspace")
    ws_deps_raw: dict[str, Any] = {}
    if isinstance(ws_table, dict):
        raw_d = ws_table.get("dependencies", {})
        if isinstance(raw_d, dict):
            ws_deps_raw = raw_d

    ws_dep_pkg_map: dict[str, str] = {}
    for key, spec in ws_deps_raw.items():
        if isinstance(spec, dict) and isinstance(spec.get("package"), str):
            ws_dep_pkg_map[key] = spec["package"]
        else:
            ws_dep_pkg_map[key] = key

    policy_support = set(policy["test_support_crates"])
    packages: list[PackageInfo] = []
    cargo_metadata_used = False

    def _collect_member_src_roots(pkg_dir: Path, is_support: bool, targets: list[dict[str, Any]]) -> tuple[list[Path], list[Path]]:
        src_roots: list[Path] = []
        prod_targets: list[Path] = []
        std_src = pkg_dir / "src"
        if std_src.is_symlink() or std_src.exists():
            src_roots.append(std_src)
        for tgt in targets:
            kinds = set(tgt.get("kind", []))
            if kinds & {"test", "example", "bench", "custom-build"} and not (
                kinds - {"test", "example", "bench", "custom-build"}
            ):
                continue
            src_path_raw = tgt.get("src_path")
            if isinstance(src_path_raw, str):
                sp = Path(os.path.realpath(src_path_raw))
                if not _is_within(sp, pkg_dir):
                    violations.append(
                        Violation(
                            "target-outside-package",
                            _to_posix_rel(pkg_dir / "Cargo.toml", root_real) if pkg_dir != root_real else "Cargo.toml",
                            1,
                            f"target src_path resolves outside package directory: {sp}",
                        )
                    )
                    continue
                if not is_support:
                    prod_targets.append(sp)
                    sp_rel = _to_posix_rel(sp, pkg_dir)
                    top = sp_rel.split("/", 1)[0]
                    if top not in ("tests", "examples", "benches", "target"):
                        if not any(_is_within(sp, sr) for sr in src_roots):
                            src_roots.append(sp.parent)
        return sorted(set(src_roots)), sorted(set(prod_targets))

    if not skip_cargo_metadata:
        try:
            proc = subprocess.run(
                ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
                cwd=root_real,
                capture_output=True,
                text=True,
                check=False,
            )
        except OSError as exc:
            violations.append(Violation("cargo-metadata-error", "Cargo.toml", 1, f"failed to invoke cargo: {exc}"))
            return [], ws_dep_pkg_map, False
        if proc.returncode != 0:
            violations.append(
                Violation(
                    "cargo-metadata-error",
                    "Cargo.toml",
                    1,
                    f"cargo metadata failed (exit {proc.returncode}): {proc.stderr.strip()}",
                )
            )
            return [], ws_dep_pkg_map, False
        try:
            meta = json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            violations.append(
                Violation("cargo-metadata-error", "Cargo.toml", 1, f"invalid cargo metadata JSON: {exc}")
            )
            return [], ws_dep_pkg_map, False

        cargo_metadata_used = True
        member_ids = set(meta.get("workspace_members", []))
        meta_pkgs = [p for p in meta.get("packages", []) if p.get("id") in member_ids]

        if isinstance(ws_table, dict) and isinstance(ws_table.get("members"), list):
            expected_dirs = _expand_workspace_globs(
                root_real,
                [m for m in ws_table["members"] if isinstance(m, str)],
                [e for e in ws_table.get("exclude", []) if isinstance(e, str)],
            )
            if isinstance(root_toml.get("package"), dict):
                expected_dirs.add(root_real)
            meta_dirs = {Path(os.path.realpath(Path(p["manifest_path"]).parent)) for p in meta_pkgs}
            if expected_dirs != meta_dirs:
                missing_in_meta = sorted(
                    _to_posix_rel(d, root_real) for d in (expected_dirs - meta_dirs) if _is_within(d, root_real)
                )
                extra_in_meta = sorted(
                    _to_posix_rel(d, root_real) for d in (meta_dirs - expected_dirs) if _is_within(d, root_real)
                )
                violations.append(
                    Violation(
                        "workspace-member-mismatch",
                        "Cargo.toml",
                        1,
                        f"cargo metadata vs Cargo.toml workspace members mismatch "
                        f"(missing={missing_in_meta}, extra={extra_in_meta})",
                    )
                )

        for p in meta_pkgs:
            manifest_path = Path(os.path.realpath(p["manifest_path"]))
            pkg_dir = manifest_path.parent
            if not _is_within(pkg_dir, root_real):
                violations.append(
                    Violation(
                        "workspace-member-outside-root",
                        "Cargo.toml",
                        1,
                        f"workspace member {p['name']} outside root: {pkg_dir}",
                    )
                )
                continue
            rel_dir = "." if pkg_dir == root_real else _to_posix_rel(pkg_dir, root_real)
            is_support = (
                rel_dir == "tests/support"
                or rel_dir.startswith("tests/support/")
                or p["name"] in policy_support
            )
            targets = list(p.get("targets", []))
            src_roots, prod_targets = _collect_member_src_roots(pkg_dir, is_support, targets)
            packages.append(
                PackageInfo(
                    name=p["name"],
                    norm_name=_norm_crate_name(p["name"]),
                    pkg_id=p["id"],
                    dir_path=pkg_dir,
                    rel_dir=rel_dir,
                    manifest_path=manifest_path,
                    is_support=is_support,
                    publish_disabled=(p.get("publish") == []),
                    src_roots=src_roots,
                    prod_target_files=prod_targets,
                    metadata_deps=list(p.get("dependencies", [])),
                    metadata_targets=targets,
                )
            )
    else:
        # Internal synthetic fixture helper path (only reachable via evaluate_workspace(..., skip_cargo_metadata=True))
        member_dirs: set[Path] = set()
        if isinstance(ws_table, dict) and isinstance(ws_table.get("members"), list):
            member_dirs |= _expand_workspace_globs(
                root_real,
                [m for m in ws_table["members"] if isinstance(m, str)],
                [e for e in ws_table.get("exclude", []) if isinstance(e, str)],
            )
        if isinstance(root_toml.get("package"), dict) or not member_dirs:
            member_dirs.add(root_real)

        for pkg_dir in sorted(member_dirs):
            manifest = pkg_dir / "Cargo.toml"
            pkg_toml = _load_toml(manifest, root_real, violations)
            if pkg_toml is None:
                continue
            pkg_sec = pkg_toml.get("package")
            if not isinstance(pkg_sec, dict) or not isinstance(pkg_sec.get("name"), str):
                continue
            pkg_name = pkg_sec["name"]
            rel_dir = "." if pkg_dir == root_real else _to_posix_rel(pkg_dir, root_real)
            is_support = (
                rel_dir == "tests/support"
                or rel_dir.startswith("tests/support/")
                or pkg_name in policy_support
            )
            pub_val = pkg_sec.get("publish")
            publish_disabled = pub_val is False or pub_val == []
            src_roots: list[Path] = []
            std_src = pkg_dir / "src"
            if std_src.is_symlink() or std_src.exists():
                src_roots.append(std_src)
            packages.append(
                PackageInfo(
                    name=pkg_name,
                    norm_name=_norm_crate_name(pkg_name),
                    pkg_id=pkg_name,
                    dir_path=pkg_dir,
                    rel_dir=rel_dir,
                    manifest_path=manifest,
                    is_support=is_support,
                    publish_disabled=publish_disabled,
                    src_roots=src_roots,
                    prod_target_files=[],
                    metadata_deps=[],
                    metadata_targets=[],
                )
            )

    packages.sort(key=lambda p: p.rel_dir)
    return packages, ws_dep_pkg_map, cargo_metadata_used


def _extract_toml_dep_sections(
    pkg_toml: dict[str, Any],
    ws_dep_pkg_map: dict[str, str],
    rel_manifest: str,
    violations: list[Violation],
) -> tuple[list[tuple[str, str, str]], list[tuple[str, str, str]]]:
    """Return (non_dev_deps, dev_deps) where each item is (section, alias_key, resolved_pkg_name)."""
    non_dev: list[tuple[str, str, str]] = []
    dev: list[tuple[str, str, str]] = []

    def _collect_table(sec_name: str, tbl: Any, dest: list[tuple[str, str, str]]) -> None:
        if not isinstance(tbl, dict):
            return
        for alias, spec in tbl.items():
            if isinstance(spec, str):
                dest.append((sec_name, alias, alias))
            elif isinstance(spec, dict):
                if spec.get("workspace") is True:
                    if alias not in ws_dep_pkg_map:
                        violations.append(
                            Violation(
                                "cargo-toml-error",
                                rel_manifest,
                                1,
                                f"[{sec_name}] {alias} uses workspace=true but is missing from [workspace.dependencies]",
                            )
                        )
                        continue
                    dest.append((sec_name, alias, ws_dep_pkg_map[alias]))
                else:
                    pkg_name = spec.get("package") if isinstance(spec.get("package"), str) else alias
                    dest.append((sec_name, alias, pkg_name))

    _collect_table("dependencies", pkg_toml.get("dependencies"), non_dev)
    _collect_table("build-dependencies", pkg_toml.get("build-dependencies"), non_dev)
    _collect_table("dev-dependencies", pkg_toml.get("dev-dependencies"), dev)

    target_tbl = pkg_toml.get("target")
    if isinstance(target_tbl, dict):
        for cfg_key, cfg_body in target_tbl.items():
            if not isinstance(cfg_body, dict):
                continue
            _collect_table(f"target.{cfg_key}.dependencies", cfg_body.get("dependencies"), non_dev)
            _collect_table(f"target.{cfg_key}.build-dependencies", cfg_body.get("build-dependencies"), non_dev)
            _collect_table(f"target.{cfg_key}.dev-dependencies", cfg_body.get("dev-dependencies"), dev)

    return non_dev, dev


def _audit_dependencies(
    packages: list[PackageInfo],
    ws_dep_pkg_map: dict[str, str],
    root_real: Path,
    cargo_metadata_used: bool,
    violations: list[Violation],
) -> tuple[set[str], dict[str, set[str]]]:
    """Validate support crate layout, aliases, workspace inheritance, non-dev leaks, and workspace DAG cycles.

    Note: `cargo metadata --no-deps` validates declared workspace-member edges and transitive closure
    within workspace packages rather than external registry crate graphs.
    """
    support_pkg_names = {p.name for p in packages if p.is_support}
    support_norm_names = {_norm_crate_name(n) for n in support_pkg_names}
    pkg_by_name = {p.name: p for p in packages}
    ws_normal_edges: dict[str, set[str]] = {p.name: set() for p in packages}
    ws_all_edges: dict[str, set[str]] = {p.name: set() for p in packages}
    support_aliases_by_pkg: dict[str, set[str]] = {p.name: set() for p in packages}

    for pkg in packages:
        rel_manifest = _to_posix_rel(pkg.manifest_path, root_real)
        pkg_toml = _load_toml(pkg.manifest_path, root_real, violations)

        if pkg.is_support:
            if not pkg.publish_disabled:
                violations.append(
                    Violation(
                        "support-crate-publishable",
                        rel_manifest,
                        1,
                        f"test-support crate '{pkg.name}' must set publish = false",
                    )
                )
            support_src = pkg.dir_path / "src"
            if support_src.is_symlink() or support_src.exists():
                rel_src = _to_posix_rel(support_src, root_real)
                violations.append(
                    Violation(
                        "support-crate-src-dir",
                        rel_src,
                        1,
                        f"test-support crate '{pkg.name}' must not contain a src/ directory or symlink",
                    )
                )
            # Enforce explicit `[lib] path = "lib.rs"` in both TOML and cargo metadata
            lib_path_ok = (
                pkg_toml is not None
                and isinstance(pkg_toml.get("lib"), dict)
                and pkg_toml["lib"].get("path") == "lib.rs"
            )
            if not lib_path_ok:
                violations.append(
                    Violation(
                        "support-crate-missing-root-lib",
                        rel_manifest,
                        1,
                        f"test-support crate '{pkg.name}' must explicitly set [lib] path = \"lib.rs\"",
                    )
                )
            if cargo_metadata_used:
                expected_lib = Path(os.path.realpath(pkg.dir_path / "lib.rs"))
                lib_targets = [t for t in pkg.metadata_targets if "lib" in t.get("kind", [])]
                if len(lib_targets) != 1 or Path(os.path.realpath(lib_targets[0].get("src_path", ""))) != expected_lib:
                    violations.append(
                        Violation(
                            "support-crate-missing-root-lib",
                            rel_manifest,
                            1,
                            f"cargo metadata reports test-support crate '{pkg.name}' lib target is not root lib.rs",
                        )
                    )

        if pkg_toml is None:
            continue
        non_dev, dev = _extract_toml_dep_sections(pkg_toml, ws_dep_pkg_map, rel_manifest, violations)

        for sec_name, alias, resolved_name in non_dev:
            resolved_norm = _norm_crate_name(resolved_name)
            if resolved_name in pkg_by_name:
                ws_normal_edges[pkg.name].add(resolved_name)
                ws_all_edges[pkg.name].add(resolved_name)
            if resolved_norm in support_norm_names or resolved_name in support_pkg_names:
                support_aliases_by_pkg[pkg.name].add(_norm_crate_name(alias))
                violations.append(
                    Violation(
                        "support-crate-in-non-dev-deps", rel_manifest, 1,
                        f"crate '{pkg.name}' depends on test-support crate "
                        f"'{resolved_name}' (alias '{alias}') in [{sec_name}]",
                    )
                )
            if pkg.is_support and resolved_name == "usdview":
                violations.append(
                    Violation(
                        "support-crate-depends-on-root",
                        rel_manifest,
                        1,
                        f"test-support crate '{pkg.name}' must not depend on root crate '{resolved_name}'",
                    )
                )

        for _sec_name, alias, resolved_name in dev:
            if resolved_name in pkg_by_name:
                ws_all_edges[pkg.name].add(resolved_name)
            if _norm_crate_name(resolved_name) in support_norm_names or resolved_name in support_pkg_names:
                support_aliases_by_pkg[pkg.name].add(_norm_crate_name(alias))
            if pkg.is_support and resolved_name == "usdview":
                violations.append(
                    Violation(
                        "support-crate-depends-on-root",
                        rel_manifest,
                        1,
                        f"test-support crate '{pkg.name}' must not depend on root crate '{resolved_name}'",
                    )
                )

        for dep in pkg.metadata_deps:
            dep_name = dep.get("name", "")
            dep_kind = dep.get("kind")
            dep_rename = dep.get("rename")
            if dep_name in pkg_by_name:
                ws_all_edges[pkg.name].add(dep_name)
                if dep_kind in (None, "build"):
                    ws_normal_edges[pkg.name].add(dep_name)
            if _norm_crate_name(dep_name) in support_norm_names or dep_name in support_pkg_names:
                if dep_rename:
                    support_aliases_by_pkg[pkg.name].add(_norm_crate_name(dep_rename))
                if dep_kind in (None, "build"):
                    violations.append(
                        Violation(
                            "support-crate-in-non-dev-deps",
                            rel_manifest,
                            1,
                            f"cargo metadata reports test-support crate '{dep_name}' "
                            f"in non-dev dependencies (kind={dep_kind or 'normal'}, rename={dep_rename})",
                        )
                    )

    # Transitive workspace non-dev closure check
    for pkg in packages:
        if pkg.is_support:
            continue
        seen: set[str] = set()
        stack = list(ws_normal_edges[pkg.name])
        while stack:
            nxt = stack.pop()
            if nxt in seen:
                continue
            seen.add(nxt)
            if nxt in support_pkg_names:
                rel_manifest = _to_posix_rel(pkg.manifest_path, root_real)
                violations.append(
                    Violation(
                        "transitive-support-crate-dep",
                        rel_manifest,
                        1,
                        f"crate '{pkg.name}' transitively reaches test-support crate '{nxt}' via workspace normal/build dependencies",
                    )
                )
            else:
                stack.extend(ws_normal_edges.get(nxt, set()))

    # Every support vertex is checked for a return path, independent of DFS order.
    # The graph is small; iterative reachability avoids recursion depth and missed SCCs.
    for support in sorted(support_pkg_names):
        stack = [(n, [support, n]) for n in sorted(ws_all_edges[support], reverse=True)]
        seen: set[str] = set()
        while stack:
            node, trail = stack.pop()
            if node == support:
                violations.append(Violation(
                    "support-architecture-cycle",
                    _to_posix_rel(pkg_by_name[support].manifest_path, root_real), 1,
                    "architectural dependency cycle: " + " -> ".join(trail),
                ))
                break
            if node not in seen:
                seen.add(node)
                stack.extend((n, trail + [n]) for n in sorted(ws_all_edges[node], reverse=True))

    return support_norm_names, support_aliases_by_pkg


def _decode_rust_escaped_string(content: str, line: int, *, is_raw: bool, prefix: str) -> tuple[str | None, str | None]:
    """Decode a Rust string literal strictly; fail closed on unsupported escapes or non-str prefixes."""
    if prefix in ("b", "br", "c", "cr"):
        return None, f"non-UTF-8/non-str literal prefix '{prefix}' is not permitted in #[path]"
    if is_raw:
        return content, None

    out: list[str] = []
    i = 0
    n = len(content)
    while i < n:
        ch = content[i]
        if ch != "\\":
            out.append(ch)
            i += 1
            continue
        i += 1
        if i >= n:
            return None, f"unterminated escape sequence at line {line}"
        esc = content[i]
        i += 1
        if esc == "n":
            out.append("\n")
        elif esc == "r":
            out.append("\r")
        elif esc == "t":
            out.append("\t")
        elif esc == "\\":
            out.append("\\")
        elif esc == '"':
            out.append('"')
        elif esc == "'":
            out.append("'")
        elif esc == "0":
            out.append("\0")
        elif esc in ("\r", "\n"):
            if esc == "\r" and i < n and content[i] == "\n":
                i += 1
            while i < n and content[i] in (" ", "\t", "\r", "\n"):
                i += 1
        elif esc == "x":
            if i + 2 > n:
                return None, f"incomplete \\xHH escape at line {line}"
            hx = content[i : i + 2]
            try:
                val = int(hx, 16)
            except ValueError:
                return None, f"invalid \\xHH escape '\\x{hx}' at line {line}"
            if val > 0x7F:
                return None, f"non-ASCII \\x{hx} escape not valid in Rust str literal at line {line}"
            out.append(chr(val))
            i += 2
        elif esc == "u":
            if i >= n or content[i] != "{":
                return None, f"malformed \\u{{...}} escape at line {line}"
            i += 1
            end = content.find("}", i)
            if end == -1:
                return None, f"unterminated \\u{{...}} escape at line {line}"
            hx = content[i:end].replace("_", "")
            if not (1 <= len(hx) <= 6):
                return None, f"invalid \\u{{{hx}}} escape length at line {line}"
            try:
                cp = int(hx, 16)
            except ValueError:
                return None, f"invalid \\u{{{hx}}} hex escape at line {line}"
            if cp > 0x10FFFF or (0xD800 <= cp <= 0xDFFF):
                return None, f"invalid Unicode scalar \\u{{{hx}}} at line {line}"
            out.append(chr(cp))
            i = end + 1
        else:
            return None, f"unsupported escape '\\{esc}' in Rust string literal at line {line}"
    return "".join(out), None


def _tokenize_rust(source: str) -> list[Token]:
    """Lex Rust source code accurately across comments, raw/escaped strings, lifetimes, and r# idents."""
    tokens: list[Token] = []
    n = len(source)
    i = 0
    line = 1
    col = 1

    def _advance(count: int = 1) -> None:
        nonlocal i, line, col
        for _ in range(count):
            if source[i] == "\n":
                line += 1
                col = 1
            else:
                col += 1
            i += 1

    def _scan_raw_string(start_line: int, start_col: int, prefix: str, hashes: int) -> Token:
        _advance(1)
        chars: list[str] = []
        closing = '"' + ("#" * hashes)
        while i < n:
            if source.startswith(closing, i):
                _advance(len(closing))
                return Token("STRING", "".join(chars), prefix + "#" * hashes + '"' + "".join(chars) + closing, start_line, start_col)
            chars.append(source[i])
            _advance(1)
        return Token("LEX_ERROR", f"unterminated raw string at line {start_line}", "", start_line, start_col)

    def _scan_quoted_string(start_line: int, start_col: int, prefix: str) -> Token:
        _advance(1)
        raw_chars: list[str] = []
        while i < n:
            ch = source[i]
            if ch == "\\":
                raw_chars.append(ch)
                _advance(1)
                if i < n:
                    raw_chars.append(source[i])
                    _advance(1)
                continue
            if ch == '"':
                _advance(1)
                body = "".join(raw_chars)
                if prefix in ("b", "c"):
                    return Token("STRING", body, prefix + '"' + body + '"', start_line, start_col)
                decoded, err = _decode_rust_escaped_string(body, start_line, is_raw=False, prefix=prefix)
                if err is not None:
                    return Token("LEX_ERROR", err, prefix + '"' + body + '"', start_line, start_col)
                return Token("STRING", decoded or "", prefix + '"' + body + '"', start_line, start_col)
            raw_chars.append(ch)
            _advance(1)
        return Token("LEX_ERROR", f"unterminated string literal at line {start_line}", "", start_line, start_col)

    while i < n:
        ch = source[i]
        if ch in (" ", "\t", "\r", "\n"):
            _advance(1)
            continue

        if ch == "/" and i + 1 < n and source[i + 1] == "/":
            _advance(2)
            while i < n and source[i] != "\n":
                _advance(1)
            continue

        if ch == "/" and i + 1 < n and source[i + 1] == "*":
            comment_start_line = line
            comment_start_col = col
            depth = 1
            _advance(2)
            while i < n and depth > 0:
                if source[i] == "/" and i + 1 < n and source[i + 1] == "*":
                    depth += 1
                    _advance(2)
                elif source[i] == "*" and i + 1 < n and source[i + 1] == "/":
                    depth -= 1
                    _advance(2)
                else:
                    _advance(1)
            if depth > 0:
                tokens.append(
                    Token(
                        "LEX_ERROR",
                        f"unterminated block comment starting at line {comment_start_line}",
                        "",
                        comment_start_line,
                        comment_start_col,
                    )
                )
                return tokens
            continue

        if ch == '"':
            tok = _scan_quoted_string(line, col, "")
            tokens.append(tok)
            if tok.kind == "LEX_ERROR":
                return tokens
            continue

        if ch in ("b", "c", "r"):
            start_line, start_col = line, col
            if ch in ("b", "c") and i + 1 < n and source[i + 1] == "r":
                j = i + 2
                hashes = 0
                while j < n and source[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and source[j] == '"':
                    _advance(j - i)
                    tok = _scan_raw_string(start_line, start_col, ch + "r", hashes)
                    tokens.append(tok)
                    if tok.kind == "LEX_ERROR":
                        return tokens
                    continue
            if ch == "r":
                j = i + 1
                hashes = 0
                while j < n and source[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and source[j] == '"':
                    _advance(j - i)
                    tok = _scan_raw_string(start_line, start_col, "r", hashes)
                    tokens.append(tok)
                    if tok.kind == "LEX_ERROR":
                        return tokens
                    continue
            if ch in ("b", "c") and i + 1 < n and source[i + 1] == '"':
                _advance(1)
                tok = _scan_quoted_string(start_line, start_col, ch)
                tokens.append(tok)
                if tok.kind == "LEX_ERROR":
                    return tokens
                continue
            if ch == "b" and i + 1 < n and source[i + 1] == "'":
                _advance(2)
                chars: list[str] = []
                if i < n and source[i] == "\\":
                    chars.append(source[i])
                    _advance(1)
                    while i < n and source[i] != "'" and source[i] != "\n":
                        chars.append(source[i])
                        _advance(1)
                elif i < n and source[i] != "'" and source[i] != "\n":
                    chars.append(source[i])
                    _advance(1)
                if i < n and source[i] == "'":
                    _advance(1)
                    tokens.append(Token("CHAR", "".join(chars), "b'" + "".join(chars) + "'", start_line, start_col))
                    continue
                tokens.append(Token("LEX_ERROR", f"unterminated byte char literal at line {start_line}", "", start_line, start_col))
                return tokens

        if source.startswith("r#", i) and i + 2 < n and (source[i + 2].isalpha() or source[i + 2] == "_"):
            start_line, start_col = line, col
            _advance(2)
            chars = ["r#"]
            while i < n and (source[i].isalnum() or source[i] == "_"):
                chars.append(source[i])
                _advance(1)
            raw = "".join(chars)
            tokens.append(Token("IDENT", _norm_ident(raw), raw, start_line, start_col))
            continue

        if ch == "'":
            start_line, start_col = line, col
            if source.startswith("'r#", i) and i + 3 < n and (source[i + 3].isalpha() or source[i + 3] == "_"):
                _advance(3)
                chars = ["'r#"]
                while i < n and (source[i].isalnum() or source[i] == "_"):
                    chars.append(source[i])
                    _advance(1)
                raw = "".join(chars)
                tokens.append(Token("LIFETIME", raw, raw, start_line, start_col))
                continue
            if i + 1 < n and source[i + 1] == "\\":
                _advance(2)
                chars = ["\\"]
                if i < n:
                    chars.append(source[i])
                    _advance(1)
                while i < n and source[i] != "'" and source[i] != "\n":
                    chars.append(source[i])
                    _advance(1)
                if i < n and source[i] == "'":
                    _advance(1)
                    tokens.append(Token("CHAR", "".join(chars), "'" + "".join(chars) + "'", start_line, start_col))
                    continue
                tokens.append(Token("LEX_ERROR", f"unterminated char literal at line {start_line}", "", start_line, start_col))
                return tokens
            if i + 2 < n and source[i + 1] != "'" and source[i + 1] != "\n" and source[i + 2] == "'":
                cval = source[i + 1]
                _advance(3)
                tokens.append(Token("CHAR", cval, f"'{cval}'", start_line, start_col))
                continue
            if i + 1 < n and (source[i + 1].isalpha() or source[i + 1] == "_"):
                _advance(1)
                chars = ["'"]
                while i < n and (source[i].isalnum() or source[i] == "_"):
                    chars.append(source[i])
                    _advance(1)
                raw = "".join(chars)
                tokens.append(Token("LIFETIME", raw, raw, start_line, start_col))
                continue
            tokens.append(Token("PUNCT", "'", "'", start_line, start_col))
            _advance(1)
            continue

        if ch.isalpha() or ch == "_":
            start_line, start_col = line, col
            chars = []
            while i < n and (source[i].isalnum() or source[i] == "_"):
                chars.append(source[i])
                _advance(1)
            raw = "".join(chars)
            tokens.append(Token("IDENT", raw, raw, start_line, start_col))
            continue

        if source.startswith("::", i):
            tokens.append(Token("PUNCT", "::", "::", line, col))
            _advance(2)
            continue

        tokens.append(Token("PUNCT", ch, ch, line, col))
        _advance(1)

    return tokens


def _parse_attr_meta_item(tokens: list[Token], idx: int, end: int) -> tuple[AttrNode, int]:
    """Parse one Rust meta item (e.g., ::tokio::test, cfg(not(test)), cfg_attr(a, b, c), path = "x")."""
    if idx >= end:
        return AttrNode("other", line=1), idx

    start_line = tokens[idx].line
    segs: list[str] = []
    if tokens[idx].kind == "PUNCT" and tokens[idx].value == "::":
        idx += 1
    while idx < end and tokens[idx].kind == "IDENT":
        segs.append(tokens[idx].value)
        idx += 1
        if idx < end and tokens[idx].kind == "PUNCT" and tokens[idx].value == "::":
            idx += 1
        else:
            break

    if not segs:
        idx += 1
        return AttrNode("other", line=start_line), idx

    seg_tuple = tuple(segs)
    if idx < end and tokens[idx].kind == "PUNCT" and tokens[idx].value == "(":
        depth = 1
        inner_start = idx + 1
        j = inner_start
        while j < end and depth > 0:
            if tokens[j].kind == "PUNCT":
                if tokens[j].value == "(":
                    depth += 1
                elif tokens[j].value == ")":
                    depth -= 1
                    if depth == 0:
                        break
            j += 1
        args: list[AttrNode] = []
        k = inner_start
        while k < j:
            if tokens[k].kind == "PUNCT" and tokens[k].value == ",":
                k += 1
                continue
            sub, k = _parse_attr_meta_item(tokens, k, j)
            args.append(sub)
        return AttrNode("call", seg_tuple, args=args, line=start_line), min(j + 1, end)

    if idx < end and tokens[idx].kind == "PUNCT" and tokens[idx].value == "=":
        idx += 1
        val_tok = tokens[idx] if idx < end else None
        if idx < end:
            if tokens[idx].kind == "PUNCT" and tokens[idx].value in ("(", "[", "{"):
                open_ch = tokens[idx].value
                close_ch = {"(": ")", "[": "]", "{": "}"}[open_ch]
                depth = 1
                idx += 1
                while idx < end and depth > 0:
                    if tokens[idx].kind == "PUNCT":
                        if tokens[idx].value == open_ch:
                            depth += 1
                        elif tokens[idx].value == close_ch:
                            depth -= 1
                    idx += 1
            else:
                idx += 1
        return AttrNode("assign", seg_tuple, value=val_tok, line=start_line), idx

    return AttrNode("path", seg_tuple, line=start_line), idx


def _cfg_expr_mentions_test(nodes: list[AttrNode]) -> tuple[bool, bool]:
    """Return (mentions_test, uses_not_test) for cfg(...) predicate argument nodes."""
    mentions = False
    has_not_test = False
    for node in nodes:
        if node.kind == "path" and node.segments == ("test",):
            mentions = True
        elif node.kind == "call":
            head = node.segments[-1] if node.segments else ""
            sub_m, sub_n = _cfg_expr_mentions_test(node.args)
            if head == "not" and sub_m:
                mentions = True
                has_not_test = True
            elif head in ("all", "any") and sub_m:
                mentions = True
                has_not_test = has_not_test or sub_n
            elif sub_m:
                mentions = True
                has_not_test = has_not_test or sub_n
    return mentions, has_not_test


def _inspect_attr_node(
    node: AttrNode,
    forbidden_suffixes: set[str],
    rel_path: str,
    violations: list[Violation],
    *,
    is_direct: bool = True,
) -> dict[str, Any]:
    """Recursively inspect an attribute AST node while preserving direct vs conditional provenance."""
    info: dict[str, Any] = {
        "node": node,
        "has_cfg_test": False,
        "is_direct_bare_cfg_test": False,
        "has_cfg_not_test": False,
        "has_cfg_attr_test": False,
        "has_forbidden_test_attr": False,
        "direct_path_values": [],
        "conditional_path_lines": [],
        "is_lint_or_doc": False,
    }
    if not node.segments:
        return info

    last = node.segments[-1]
    is_cfg = last == "cfg" and (len(node.segments) == 1 or node.segments in (("core", "cfg"), ("std", "cfg")))
    is_cfg_attr = last == "cfg_attr" and (
        len(node.segments) == 1 or node.segments in (("core", "cfg_attr"), ("std", "cfg_attr"))
    )

    if is_cfg and node.kind == "call":
        mentions, has_not = _cfg_expr_mentions_test(node.args)
        if has_not:
            info["has_cfg_not_test"] = True
        if mentions:
            info["has_cfg_test"] = True
            if (
                is_direct
                and not has_not
                and node.segments == ("cfg",)
                and len(node.args) == 1
                and node.args[0].kind == "path"
                and node.args[0].segments == ("test",)
            ):
                info["is_direct_bare_cfg_test"] = True
        return info

    if is_cfg_attr and node.kind == "call":
        if node.args:
            mentions, _ = _cfg_expr_mentions_test([node.args[0]])
            if mentions:
                info["has_cfg_attr_test"] = True
                violations.append(
                    Violation(
                        "forbidden-cfg-attr-test",
                        rel_path,
                        node.line,
                        "#[cfg_attr(test, ...)] is forbidden under src/",
                    )
                )
            for payload in node.args[1:]:
                sub = _inspect_attr_node(payload, forbidden_suffixes, rel_path, violations, is_direct=False)
                if sub["has_cfg_test"]:
                    info["has_cfg_test"] = True
                    violations.append(
                        Violation(
                            "forbidden-cfg-attr-nested-cfg-test",
                            rel_path,
                            payload.line,
                            "#[cfg_attr(..., cfg(test))] is forbidden under src/",
                        )
                    )
                if sub["has_cfg_not_test"]:
                    info["has_cfg_not_test"] = True
                if sub["has_cfg_attr_test"]:
                    info["has_cfg_attr_test"] = True
                if sub["has_forbidden_test_attr"]:
                    info["has_forbidden_test_attr"] = True
                for pline, _ in sub["direct_path_values"]:
                    info["conditional_path_lines"].append(pline)
                info["conditional_path_lines"].extend(sub["conditional_path_lines"])
        return info

    if node.segments == ("path",) and node.kind == "assign":
        if is_direct:
            info["direct_path_values"].append((node.line, node.value))
        else:
            info["conditional_path_lines"].append(node.line)
        return info

    if is_direct and last in ALLOWED_TEST_MOD_EXTRA_ATTRS:
        info["is_lint_or_doc"] = True

    if last in forbidden_suffixes:
        info["has_forbidden_test_attr"] = True
        violations.append(
            Violation(
                "forbidden-test-attribute",
                rel_path,
                node.line,
                f"forbidden test attribute #[{'::'.join(node.segments)}] under src/",
            )
        )
    return info


def _resolve_mod_path_target(
    attr_line: int,
    tok: Token | None,
    base_dir: Path,
    root_real: Path,
    src_roots: list[Path],
    rel_owner: str,
    violations: list[Violation],
    *,
    expect_outside_src: bool,
) -> Path | None:
    if tok is None or tok.kind != "STRING":
        violations.append(
            Violation("invalid-path-attribute", rel_owner, attr_line, "#[path] must be a valid string literal")
        )
        return None
    if tok.raw.startswith(('b"', 'br"', "br#", 'c"', 'cr"', "cr#")):
        violations.append(
            Violation(
                "invalid-path-attribute",
                rel_owner,
                attr_line,
                "#[path] must be a UTF-8 string literal (byte/C strings forbidden)",
            )
        )
        return None

    candidate = base_dir / tok.value
    resolved = Path(os.path.realpath(candidate))
    if not _is_within(resolved, root_real):
        violations.append(
            Violation(
                "path-escapes-workspace",
                rel_owner,
                attr_line,
                f"#[path = {tok.value!r}] resolves outside workspace root: {resolved}",
            )
        )
        return None
    target_dir = Path(os.path.realpath(root_real / "target"))
    if _is_within(resolved, target_dir):
        violations.append(
            Violation(
                "path-inside-target",
                rel_owner,
                attr_line,
                f"#[path = {tok.value!r}] resolves inside target/: {_to_posix_rel(resolved, root_real)}",
            )
        )
        return None
    if candidate.is_symlink() and not resolved.exists():
        violations.append(
            Violation("broken-symlink", rel_owner, attr_line, f"#[path = {tok.value!r}] points to broken symlink")
        )
        return None
    if not resolved.is_file():
        violations.append(
            Violation(
                "missing-path-target",
                rel_owner,
                attr_line,
                f"#[path = {tok.value!r}] target file does not exist: {_to_posix_rel(resolved, root_real)}",
            )
        )
        return None

    inside_src = any(_is_within(resolved, Path(os.path.realpath(sr))) for sr in src_roots)
    if expect_outside_src and inside_src:
        violations.append(
            Violation(
                "cfg-test-path-inside-src",
                rel_owner,
                attr_line,
                f"#[cfg(test)] #[path = {tok.value!r}] resolves inside src/: {_to_posix_rel(resolved, root_real)}",
            )
        )
        return None
    if not expect_outside_src and not inside_src:
        violations.append(
            Violation(
                "prod-path-outside-src",
                rel_owner,
                attr_line,
                f"production #[path = {tok.value!r}] resolves outside src/: {_to_posix_rel(resolved, root_real)}",
            )
        )
        return None
    return resolved


def _extract_use_tree_roots(tokens: list[Token], start: int) -> tuple[list[tuple[str, int]], int]:
    """Parse a `use <tree>;` statement and return leading root crate/module idents with lines."""
    n = len(tokens)
    roots: list[tuple[str, int]] = []
    i = start
    expecting_root = True
    brace_depth = 0
    root_at_depth: dict[int, bool] = {0: True}

    while i < n:
        tok = tokens[i]
        if tok.kind == "PUNCT" and tok.value == ";" and brace_depth == 0:
            return roots, i
        if tok.kind == "PUNCT" and tok.value == "{":
            brace_depth += 1
            root_at_depth[brace_depth] = expecting_root
        elif tok.kind == "PUNCT" and tok.value == "}":
            brace_depth = max(0, brace_depth - 1)
            expecting_root = False
        elif tok.kind == "PUNCT" and tok.value == ",":
            expecting_root = root_at_depth.get(brace_depth, False)
        elif tok.kind == "PUNCT" and tok.value == "::":
            # Leading `::foo` at statement/group start still starts a crate root; `a::b` suppresses root for `b`
            pass
        elif tok.kind == "IDENT":
            if tok.value == "as":
                # Skip alias ident until comma/brace/semicolon
                expecting_root = False
                i += 1
                if i < n and tokens[i].kind == "IDENT":
                    i += 1
                continue
            if expecting_root:
                if tok.value not in ("crate", "self", "super"):
                    roots.append((tok.value, tok.line))
                expecting_root = False
        i += 1
    return roots, i


def _scan_src_rs_file(
    file_path: Path,
    root_real: Path,
    src_roots: list[Path],
    forbidden_suffixes: set[str],
    forbidden_aliases: set[str],
    violations: list[Violation],
    *,
    external_inventory: bool = False,
    crate_root: bool = False,
) -> tuple[list[AttachmentRecord], list[ModBinding], list[IncludeMarker], list[Token], bool]:
    """Scan one .rs file under src/.

    Returns (attachments, mod_bindings, include_markers, prod_tokens, ok).
    """
    rel_path = _to_posix_rel(file_path, root_real)
    source = _read_text_strict(file_path, rel_path, violations)
    if source is None:
        return [], [], [], [], False

    tokens = _tokenize_rust(source)
    for tok in tokens:
        if tok.kind == "LEX_ERROR":
            violations.append(Violation("unsupported-rust-syntax", rel_path, tok.line, tok.value))
            return [], [], [], [], False

    attachments: list[AttachmentRecord] = []
    mod_bindings: list[ModBinding] = []
    include_markers: list[IncludeMarker] = []
    prod_tokens: list[Token] = []

    initial_path_base = file_path.parent
    initial_inline_child_base = (
        file_path.parent
        if crate_root or file_path.name in ("lib.rs", "main.rs", "mod.rs")
        else file_path.parent / file_path.stem
    )
    dir_stack: list[tuple[Path, Path]] = [(initial_path_base, initial_inline_child_base)]
    mod_name_stack: list[str] = []
    block_stack: list[bool] = []  # True iff block opened an inline `mod`

    pending_attrs: list[dict[str, Any]] = []
    # Track token index where current pending_attrs started for clean production-token stripping
    pending_start_prod_idx: int | None = None
    paren_depth = 0
    bracket_depth = 0

    def _flush_non_path_attr_check(item_desc: str) -> None:
        nonlocal pending_attrs, pending_start_prod_idx
        for attr in pending_attrs:
            if attr["has_cfg_not_test"]:
                violations.append(
                    Violation(
                        "forbidden-cfg-not-test",
                        rel_path,
                        attr["line"],
                        f"#[cfg(not(test))] is forbidden on {item_desc} under src/",
                    )
                )
            elif attr["has_cfg_test"]:
                violations.append(
                    Violation(
                        "forbidden-non-path-cfg-test",
                        rel_path,
                        attr["line"],
                        f"non-path #[cfg(test)] on {item_desc} is forbidden under src/",
                    )
                )
        pending_attrs = []
        pending_start_prod_idx = None

    lexical_events = list(_scope_tokens(tokens))
    actual_tokens = {i for _, _, i, _, _ in lexical_events}
    actual_modules = {i for scope, blocks, i, kind, value in lexical_events
                      if len(blocks) == len(scope) and
                      (kind == "module" or kind == "token" and value.kind == "IDENT" and value.value == "mod")}
    n = len(tokens)
    i = 0
    while i < n:
        tok = tokens[i]

        # Outer or inner attributes: #[...] or #![...]
        if tok.kind == "PUNCT" and tok.value == "#":
            is_inner = i + 1 < n and tokens[i + 1].kind == "PUNCT" and tokens[i + 1].value == "!"
            open_idx = i + 2 if is_inner else i + 1
            if open_idx < n and tokens[open_idx].kind == "PUNCT" and tokens[open_idx].value == "[":
                depth = 1
                j = open_idx + 1
                while j < n and depth > 0:
                    if tokens[j].kind == "PUNCT":
                        if tokens[j].value == "[":
                            depth += 1
                        elif tokens[j].value == "]":
                            depth -= 1
                            if depth == 0:
                                break
                    j += 1
                if depth != 0:
                    violations.append(
                        Violation("malformed-attribute", rel_path, tok.line, "unclosed #[...] attribute")
                    )
                    return attachments, mod_bindings, include_markers, prod_tokens, False

                meta, _ = _parse_attr_meta_item(tokens, open_idx + 1, j)
                info = _inspect_attr_node(meta, forbidden_suffixes, rel_path, violations, is_direct=True)
                info["line"] = tok.line
                if i not in actual_tokens and (info["has_cfg_test"] or info["has_cfg_not_test"] or info["has_cfg_attr_test"]):
                    violations.append(Violation("non-direct-test-module-attachment", rel_path, tok.line,
                                                "test configuration in opaque token tree cannot prove a direct attachment"))
                if is_inner:
                    if info["has_cfg_not_test"] or info["has_cfg_test"]:
                        violations.append(
                            Violation(
                                "forbidden-inner-cfg-test",
                                rel_path,
                                tok.line,
                                "inner #![cfg(test)] / #![cfg(not(test))] is forbidden under src/",
                            )
                        )
                else:
                    if pending_start_prod_idx is None:
                        pending_start_prod_idx = len(prod_tokens)
                    pending_attrs.append(info)
                i = j + 1
                continue

        # Detect cfg!(...), cfg![...], cfg!{...} including ::core::cfg! / std::cfg!
        if (
            tok.kind == "IDENT"
            and tok.value == "cfg"
            and i + 2 < n
            and tokens[i + 1].kind == "PUNCT"
            and tokens[i + 1].value == "!"
            and tokens[i + 2].kind == "PUNCT"
            and tokens[i + 2].value in ("(", "[", "{")
        ):
            # Ignore `foo.cfg!()` field-like unlikely forms? No, cfg! only preceded by path/space
            open_delim = tokens[i + 2].value
            close_delim = {"(": ")", "[": "]", "{": "}"}[open_delim]
            depth = 1
            j = i + 3
            while j < n and depth > 0:
                if tokens[j].kind == "PUNCT":
                    if tokens[j].value == open_delim:
                        depth += 1
                    elif tokens[j].value == close_delim:
                        depth -= 1
                        if depth == 0:
                            break
                j += 1
            if depth != 0:
                violations.append(
                    Violation("malformed-cfg-macro", rel_path, tok.line, "unclosed cfg! macro invocation")
                )
                return attachments, mod_bindings, include_markers, prod_tokens, False
            args: list[AttrNode] = []
            k = i + 3
            while k < j:
                if tokens[k].kind == "PUNCT" and tokens[k].value == ",":
                    k += 1
                    continue
                sub, k = _parse_attr_meta_item(tokens, k, j)
                args.append(sub)
            mentions, _ = _cfg_expr_mentions_test(args)
            if mentions:
                violations.append(
                    Violation("forbidden-cfg-macro-test", rel_path, tok.line, f"cfg!{open_delim}...{close_delim} referencing test is forbidden under src/")
                )
            prod_tokens.append(tok)
            i += 1
            continue

        # Detect include!(...) and env!("OUT_DIR") with exact expression/target extraction
        if (
            tok.kind == "IDENT"
            and tok.value == "include"
            and i + 2 < n
            and tokens[i + 1].kind == "PUNCT"
            and tokens[i + 1].value == "!"
            and tokens[i + 2].kind == "PUNCT"
            and tokens[i + 2].value in ("(", "[", "{")
        ):
            args, end = _call_args(tokens, i + 2)
            if args is None or len(args) != 1:
                include_markers.append(IncludeMarker(rel_path, tok.line, "include_dynamic", None, None))
            else:
                package_dir = next((p for p in file_path.parents if _is_within(p, root_real) and (p / "Cargo.toml").is_file()), None)
                value = _path_expression(args[0], file_path.parent, package_dir, {})
                if isinstance(value, Path):
                    include_markers.append(IncludeMarker(rel_path, tok.line, "include_literal", str(value), value))
                elif isinstance(value, str) and value.startswith("OUT_DIR:"):
                    include_markers.append(IncludeMarker(rel_path, tok.line, "include_out_dir", value[8:], None))
                else:
                    include_markers.append(IncludeMarker(rel_path, tok.line, "include_dynamic", None, None))
            prod_tokens.extend(tokens[i:end])
            i = end
            continue
        elif (
            tok.kind == "IDENT"
            and tok.value == "env"
            and i + 3 < n
            and tokens[i + 1].kind == "PUNCT"
            and tokens[i + 1].value == "!"
            and tokens[i + 2].kind == "PUNCT"
            and tokens[i + 2].value == "("
            and tokens[i + 3].kind == "STRING"
            and tokens[i + 3].value == "OUT_DIR"
        ):
            # Stand-alone env!("OUT_DIR") outside include!(...)
            include_markers.append(IncludeMarker(rel_path, tok.line, "out_dir", None, None))

        # Detect support crate imports (grouped use, leading ::, extern crate) and qualified path calls
        if paren_depth == 0 and bracket_depth == 0 and tok.kind == "IDENT":
            if tok.value == "extern" and i + 2 < n and tokens[i + 1].kind == "IDENT" and tokens[i + 1].value == "crate":
                alias_tok = tokens[i + 2]
                if alias_tok.kind == "IDENT" and alias_tok.value in forbidden_aliases:
                    violations.append(
                        Violation(
                            "support-crate-import-in-src",
                            rel_path,
                            alias_tok.line,
                            f"extern crate '{alias_tok.value}' imports a test-support crate under src/",
                        )
                    )
            elif tok.value == "use":
                use_roots, _ = _extract_use_tree_roots(tokens, i + 1)
                for root_ident, root_line in use_roots:
                    if root_ident in forbidden_aliases:
                        violations.append(
                            Violation(
                                "support-crate-import-in-src",
                                rel_path,
                                root_line,
                                f"use import of test-support crate '{root_ident}' is forbidden under src/",
                            )
                        )

        if (
            tok.kind == "IDENT"
            and tok.value in forbidden_aliases
            and i + 1 < n
            and tokens[i + 1].kind == "PUNCT"
            and tokens[i + 1].value == "::"
        ):
            prev_is_rel_sep = (
                i > 0
                and tokens[i - 1].kind == "PUNCT"
                and tokens[i - 1].value == "::"
                and i > 1
                and tokens[i - 2].kind == "IDENT"
            )
            prev_is_dot = i > 0 and tokens[i - 1].kind == "PUNCT" and tokens[i - 1].value == "."
            if not prev_is_rel_sep and not prev_is_dot:
                violations.append(
                    Violation(
                        "support-crate-import-in-src",
                        rel_path,
                        tok.line,
                        f"qualified path expression/type referencing test-support crate '{tok.value}' is forbidden under src/",
                    )
                )

        if tok.kind == "PUNCT":
            if tok.value == "(":
                paren_depth += 1
            elif tok.value == ")":
                paren_depth = max(0, paren_depth - 1)
            elif tok.value == "[":
                bracket_depth += 1
            elif tok.value == "]":
                bracket_depth = max(0, bracket_depth - 1)

        # Module declarations at item level (outside (...) and [...])
        if paren_depth == 0 and bracket_depth == 0 and tok.kind == "IDENT" and tok.value == "mod":
            prev_is_path = i > 0 and tokens[i - 1].kind == "PUNCT" and tokens[i - 1].value in ("::", ".")
            if not prev_is_path and i + 2 < n and tokens[i + 1].kind == "IDENT":
                mod_name = tokens[i + 1].value
                delim = tokens[i + 2]
                if delim.kind == "PUNCT" and delim.value == ";":
                    if i not in actual_modules:
                        if any(a["has_cfg_test"] or a["has_cfg_not_test"] or a["has_cfg_attr_test"] for a in pending_attrs):
                            violations.append(Violation("non-direct-test-module-attachment", rel_path, tok.line,
                                                        "test module tokens are not a direct module item"))
                        pending_attrs = []
                        pending_start_prod_idx = None
                        i += 3
                        continue
                    if external_inventory:
                        # Reuse module-context resolution for already external test
                        # harnesses. Strict production attachment rules do not apply here.
                        direct_paths = [pv for a in pending_attrs for pv in a["direct_path_values"]]
                        conditional = any(a["conditional_path_lines"] or a["has_cfg_not_test"] for a in pending_attrs)
                        resolved = None
                        if not conditional and len(direct_paths) == 1:
                            attr_line, ptok = direct_paths[0]
                            resolved = _resolve_mod_path_target(
                                attr_line, ptok, dir_stack[-1][0], root_real,
                                src_roots, rel_path, violations, expect_outside_src=True,
                            )
                        elif not conditional and not direct_paths:
                            _, child_base = dir_stack[-1]
                            candidates = [child_base / f"{mod_name}.rs", child_base / mod_name / "mod.rs"]
                            existing = [Path(os.path.realpath(p)) for p in candidates if p.is_file()]
                            if len(existing) == 1 and _is_within(existing[0], root_real) and not any(_is_within(existing[0], sr) for sr in src_roots):
                                resolved = existing[0]
                        if resolved is not None:
                            mod_bindings.append(ModBinding(rel_path, mod_name, resolved, tok.line, tuple(mod_name_stack)))
                        pending_attrs = []
                        pending_start_prod_idx = None
                        i += 3
                        continue
                    has_any_cfg_test = any(
                        a["has_cfg_test"] or a["has_cfg_not_test"] or a["has_cfg_attr_test"]
                        for a in pending_attrs
                    )
                    if has_any_cfg_test:
                        direct_bare_cfg_tests = [a for a in pending_attrs if a["is_direct_bare_cfg_test"]]
                        non_bare_cfg_tests = [
                            a
                            for a in pending_attrs
                            if (a["has_cfg_test"] or a["has_cfg_not_test"] or a["has_cfg_attr_test"])
                            and not a["is_direct_bare_cfg_test"]
                        ]
                        direct_paths = [pv for a in pending_attrs for pv in a["direct_path_values"]]
                        cond_path_lines = [ln for a in pending_attrs for ln in a["conditional_path_lines"]]
                        extra_attrs = [
                            a
                            for a in pending_attrs
                            if not a["is_direct_bare_cfg_test"]
                            and not a["direct_path_values"]
                            and not a["is_lint_or_doc"]
                            and not (a["has_cfg_test"] or a["has_cfg_not_test"] or a["has_cfg_attr_test"] or a["conditional_path_lines"])
                        ]

                        valid_strict_attachment = True
                        if any(a["has_cfg_not_test"] for a in pending_attrs):
                            violations.append(
                                Violation(
                                    "forbidden-cfg-not-test",
                                    rel_path,
                                    tok.line,
                                    f"#[cfg(not(test))] mod {mod_name}; is forbidden under src/",
                                )
                            )
                            valid_strict_attachment = False
                        if non_bare_cfg_tests:
                            violations.append(
                                Violation(
                                    "non-exclusive-cfg-test-attachment",
                                    rel_path,
                                    non_bare_cfg_tests[0]["line"],
                                    f"external test mod {mod_name}; must use direct #[cfg(test)] only (compound/conditional cfg is forbidden)",
                                )
                            )
                            valid_strict_attachment = False
                        elif len(direct_bare_cfg_tests) != 1:
                            violations.append(
                                Violation(
                                    "non-exclusive-cfg-test-attachment",
                                    rel_path,
                                    tok.line,
                                    f"external test mod {mod_name}; must have strictly one direct #[cfg(test)] attribute",
                                )
                            )
                            valid_strict_attachment = False
                        if cond_path_lines:
                            violations.append(
                                Violation(
                                    "conditional-path-on-test-mod",
                                    rel_path,
                                    cond_path_lines[0],
                                    f"#[cfg(test)] mod {mod_name}; must use an unconditional direct #[path = \"...\"]",
                                )
                            )
                            valid_strict_attachment = False
                        if not direct_paths:
                            if not cond_path_lines:
                                violations.append(
                                    Violation(
                                        "non-path-cfg-test-mod",
                                        rel_path,
                                        tok.line,
                                        f"#[cfg(test)] mod {mod_name}; under src/ requires explicit direct #[path = \"...\"]",
                                    )
                                )
                            valid_strict_attachment = False
                        elif len(direct_paths) > 1:
                            violations.append(
                                Violation(
                                    "duplicate-path-attribute",
                                    rel_path,
                                    direct_paths[1][0],
                                    f"multiple #[path] attributes on #[cfg(test)] mod {mod_name};",
                                )
                            )
                            valid_strict_attachment = False
                        if extra_attrs:
                            violations.append(
                                Violation(
                                    "forbidden-extra-attr-on-test-mod",
                                    rel_path,
                                    extra_attrs[0]["line"],
                                    f"unexpected non-lint attribute on #[cfg(test)] mod {mod_name};",
                                )
                            )
                            valid_strict_attachment = False

                        if valid_strict_attachment:
                            attr_line, ptok = direct_paths[0]
                            base_dir, _ = dir_stack[-1]
                            resolved = _resolve_mod_path_target(
                                attr_line,
                                ptok,
                                base_dir,
                                root_real,
                                src_roots,
                                rel_path,
                                violations,
                                expect_outside_src=True,
                            )
                            if resolved is not None:
                                attachments.append(
                                    AttachmentRecord(
                                        owner_file=rel_path,
                                        module_path="::".join(mod_name_stack),
                                        mod_name=mod_name,
                                        target=_to_posix_rel(resolved, root_real),
                                        line=tok.line,
                                    )
                                )

                        # Strip this #[cfg(test)] attachment item from prod_tokens cleanly
                        if pending_start_prod_idx is not None:
                            del prod_tokens[pending_start_prod_idx:]
                    else:
                        direct_paths = [pv for a in pending_attrs for pv in a["direct_path_values"]]
                        cond_path_lines = [ln for a in pending_attrs for ln in a["conditional_path_lines"]]
                        if direct_paths:
                            attr_line, ptok = direct_paths[-1]
                            base_dir, _ = dir_stack[-1]
                            resolved = _resolve_mod_path_target(
                                attr_line,
                                ptok,
                                base_dir,
                                root_real,
                                src_roots,
                                rel_path,
                                violations,
                                expect_outside_src=False,
                            )
                            if resolved is not None:
                                mod_bindings.append(ModBinding(rel_path, mod_name, Path(os.path.realpath(resolved)), tok.line, tuple(mod_name_stack)))
                        elif not cond_path_lines:
                            _, child_base = dir_stack[-1]
                            cand_rs = child_base / f"{mod_name}.rs"
                            cand_mod = child_base / mod_name / "mod.rs"
                            if cand_rs.is_file():
                                mod_bindings.append(ModBinding(rel_path, mod_name, Path(os.path.realpath(cand_rs)), tok.line, tuple(mod_name_stack)))
                            elif cand_mod.is_file():
                                mod_bindings.append(ModBinding(rel_path, mod_name, Path(os.path.realpath(cand_mod)), tok.line, tuple(mod_name_stack)))
                        if mod_name in FORBIDDEN_INLINE_MOD_EXACT or mod_name.endswith("_tests"):
                            violations.append(
                                Violation(
                                    "forbidden-test-module-name",
                                    rel_path,
                                    tok.line,
                                    f"module declaration 'mod {mod_name};' uses forbidden test module name under src/",
                                )
                            )
                    pending_attrs = []
                    pending_start_prod_idx = None
                    i += 3
                    continue

                if delim.kind == "PUNCT" and delim.value == "{":
                    has_cfg_test = any(a["has_cfg_test"] or a["has_cfg_not_test"] or a["has_cfg_attr_test"] for a in pending_attrs)
                    path_attrs = [pv for a in pending_attrs for pv in a["direct_path_values"]]
                    if has_cfg_test:
                        violations.append(
                            Violation(
                                "forbidden-inline-cfg-test",
                                rel_path,
                                tok.line,
                                f"inline #[cfg(test)] mod {mod_name} {{ ... }} is forbidden under src/",
                            )
                        )
                    if mod_name in FORBIDDEN_INLINE_MOD_EXACT or mod_name.endswith("_tests"):
                        violations.append(
                            Violation(
                                "forbidden-inline-test-module",
                                rel_path,
                                tok.line,
                                f"inline test module 'mod {mod_name} {{ ... }}' is forbidden under src/",
                            )
                        )

                    cur_path_base, cur_inline_base = dir_stack[-1]
                    if path_attrs:
                        attr_line, ptok = path_attrs[-1]
                        if ptok is None or ptok.kind != "STRING" or ptok.raw.startswith(
                            ('b"', 'br"', "br#", 'c"', 'cr"', "cr#")
                        ):
                            violations.append(
                                Violation(
                                    "invalid-path-attribute",
                                    rel_path,
                                    attr_line,
                                    "inline mod #[path] must be a valid UTF-8 string literal",
                                )
                            )
                            new_dir = cur_inline_base / mod_name
                        else:
                            new_dir = cur_path_base / ptok.value
                    else:
                        new_dir = cur_inline_base / mod_name

                    dir_stack.append((new_dir, new_dir))
                    mod_name_stack.append(mod_name)
                    block_stack.append(True)
                    pending_attrs = []
                    pending_start_prod_idx = None
                    prod_tokens.extend([tok, tokens[i + 1], delim])
                    i += 3
                    continue

        if paren_depth == 0 and bracket_depth == 0 and tok.kind == "PUNCT":
            if tok.value == "{":
                if pending_attrs:
                    _flush_non_path_attr_check("item/block")
                block_stack.append(False)
            elif tok.value == "}":
                if pending_attrs:
                    _flush_non_path_attr_check("trailing item")
                if block_stack:
                    was_mod = block_stack.pop()
                    if was_mod:
                        if len(dir_stack) > 1:
                            dir_stack.pop()
                        if mod_name_stack:
                            mod_name_stack.pop()
            elif tok.value in (";", ","):
                if pending_attrs:
                    _flush_non_path_attr_check("statement/field")

        prod_tokens.append(tok)
        i += 1

    if pending_attrs:
        _flush_non_path_attr_check("trailing item")

    return attachments, mod_bindings, include_markers, prod_tokens, True


def _is_candidate_fixture_name(path: Path) -> bool:
    if path.suffix != ".rs":
        return True
    stem = path.stem.lower()
    return any(kw in stem for kw in CANDIDATE_FIXTURE_KEYWORDS)


def _call_args(tokens: list[Token], opening: int) -> tuple[list[list[Token]] | None, int]:
    """Split one balanced token tree at top-level commas; reject mismatched delimiters."""
    pairs = {"(": ")", "[": "]", "{": "}"}
    if opening >= len(tokens) or tokens[opening].kind != "PUNCT" or tokens[opening].value not in pairs:
        return None, opening + 1
    stack = [pairs[tokens[opening].value]]
    args: list[list[Token]] = []
    start = opening + 1
    for i in range(start, len(tokens)):
        t = tokens[i]
        if t.kind != "PUNCT":
            continue
        if t.value in pairs:
            stack.append(pairs[t.value])
        elif t.value in pairs.values():
            if t.value != stack.pop():
                return None, i + 1
            if not stack:
                if start < i:
                    args.append(tokens[start:i])
                return args, i + 1
        elif t.value == "," and len(stack) == 1:
            args.append(tokens[start:i])
            start = i + 1
    return None, len(tokens)


def _values(tokens: list[Token]) -> list[str]:
    return [t.value for t in tokens]


def _literal(tokens: list[Token]) -> str | None:
    if len(tokens) == 1 and tokens[0].kind == "STRING" and not tokens[0].raw.startswith(("b", "c")):
        return tokens[0].value
    return None


def _relative(value: str) -> str | None:
    # OUT_DIR identities are package-relative and cannot traverse out of that root.
    if not value or "\\" in value or ":" in value or value.startswith("/"):
        return None
    parts = value.split("/")
    if any(p in ("", ".", "..") for p in parts):
        return None
    return "/".join(parts)


def _path_expression(tokens: list[Token], base: Path, package: Path | None,
                     bindings: dict[str, Path | str], *, context: _ProofContext | None = None,
                     at: int = 0, joinable: frozenset[str] = frozenset()) -> Path | str | None:
    """Resolve the whole supported path expression, never individual embedded strings.

    A symbolic OUT_DIR: path is anchored to the consumer/producer package later.
    Unsupported Rust expressions are deliberately not treated as proof.
    """
    v = _values(tokens)
    if v[:1] == ["&"]:
        return _path_expression(tokens[1:], base, package, bindings, context=context, at=at, joinable=joinable)
    literal = _literal(tokens)
    if literal is not None:
        return Path(os.path.realpath(base / literal))
    if len(v) == 1 and tokens[0].kind == "IDENT":
        return bindings.get(v[0])
    # Exact concat!(env!(...), "suffix") or concatenated literal strings.
    if v[:2] == ["concat", "!"]:
        if context is not None and context.macro_name(("concat",), at) != "concat":
            return None
        args, end = _call_args(tokens, 2)
        if args is None or end != len(tokens) or not args:
            return None
        parts: list[str] = []
        for i, arg in enumerate(args):
            lit = _literal(arg)
            if lit is not None:
                parts.append(lit)
            elif i == 0 and _values(arg)[:2] == ["env", "!"]:
                if context is not None and context.macro_name(("env",), at) != "env":
                    return None
                inner, stop = _call_args(arg, 2)
                env = _literal(inner[0]) if inner and len(inner) == 1 and stop == len(arg) else None
                if env == "OUT_DIR":
                    parts.append("OUT_DIR:")
                elif env == "CARGO_MANIFEST_DIR" and package is not None:
                    parts.append(str(package))
                else:
                    return None
            else:
                return None
        value = "".join(parts)
        if value.startswith("OUT_DIR:"):
            tail = value[8:]
            if not tail.startswith("/"):
                return None
            rel = _relative(tail[1:])
            return "OUT_DIR:" + rel if rel else None
        return Path(os.path.realpath(base / value))
    # PathBuf::from(env!("CARGO_MANIFEST_DIR")) and OUT_DIR equivalents.
    constructor = next((j for j in range(len(v)) if v[j] == "("), -1)
    constructor_path = (("::",) if v[:1] == ["::"] else ()) + tuple(x for x in v[:constructor] if x != "::") if constructor >= 0 else ()
    if constructor_path and constructor_path[-1] in {"from", "new"} and all(
        t.kind == "IDENT" or t.value == "::" for t in tokens[:constructor]
    ):
        expected = {("std", "path", "PathBuf", "from"), ("std", "path", "Path", "new")}
        if context is None:
            if constructor_path not in {("PathBuf", "from"), ("Path", "new")}:
                return None
        elif context.identity(constructor_path, at) not in expected:
            return None
        args, end = _call_args(tokens, constructor)
        if args is None or len(args) != 1:
            return None
        arg = args[0]
        av = _values(arg)
        if av[:2] == ["env", "!"]:
            if context is not None and context.macro_name(("env",), at) != "env":
                return None
            inner, stop = _call_args(arg, 2)
            env = _literal(inner[0]) if inner and len(inner) == 1 and stop == len(arg) else None
            result = package if env == "CARGO_MANIFEST_DIR" else "OUT_DIR:" if env == "OUT_DIR" else None
        elif av[:6] == ["std", "::", "env", "::", "var", "("] or av[:4] == ["env", "::", "var", "("]:
            opening = 5 if av[0] == "std" else 3
            if context is not None and context.identity(tuple(x for x in av[:opening] if x != "::"), at) != ("std", "env", "var"):
                return None
            inner, stop = _call_args(arg, opening)
            env = _literal(inner[0]) if inner and len(inner) == 1 and av[stop:] == [".", "unwrap", "(", ")"] else None
            result = "OUT_DIR:" if env == "OUT_DIR" else package if env == "CARGO_MANIFEST_DIR" else None
        else:
            result = _path_expression(arg, base, package, bindings, context=context, at=at, joinable=joinable)
    elif tokens and tokens[0].kind == "IDENT" and v[0] in bindings:
        if context is not None and len(v) > 1 and v[0] not in joinable:
            return None
        result, end = bindings[v[0]], 1
    else:
        return None
    while result is not None and end < len(tokens):
        if v[end:end+3] != [".", "join", "("]:
            return None
        args, stop = _call_args(tokens, end + 2)
        part = _literal(args[0]) if args and len(args) == 1 else None
        if part is None:
            return None
        if isinstance(result, Path):
            result = Path(os.path.realpath(result / part))
        else:
            rel = _relative(part)
            if not rel:
                return None
            previous = result[8:]
            result = "OUT_DIR:" + (previous + "/" if previous else "") + rel
        end = stop
    return result


def _macro_invocations(tokens: list[Token]):
    for scope, blocks, i, kind, value in _scope_tokens(tokens):
        if kind == "macro":
            path, args = value
            yield i, path, args


_BUILTIN_MACROS = frozenset({"include", "include_str", "include_bytes", "concat", "env"})


class _ProofContext:
    """Bounded lexical identity model shared by asset and generation proofs.

    Only explicit imports, standard roots and declared dependency aliases are
    admitted. Local or ambiguous bindings cannot establish external identity.
    Unsupported re-exports/globs are not guessed. This is not macro expansion.
    """
    def __init__(self, tokens: list[Token], pkg: PackageInfo | None = None):
        self.tokens = tokens
        self.events = list(_scope_tokens(tokens))
        self.locations = {i: (scope, blocks) for scope, blocks, i, _, _ in self.events}
        self.imports = []
        self.shadows = []
        self.macros = []
        self.externals = {"std": "std", "core": "core"}
        if pkg is not None:
            for dep in pkg.metadata_deps:
                if dep.get("kind") is None:
                    name = dep.get("name", "")
                    alias = _norm_crate_name(dep.get("rename") or name)
                    if alias not in {"std", "core"}:
                        self.externals[alias] = name
            # Synthetic test fixtures have no Cargo metadata. Their declared
            # direct manifest dependencies supply the same limited identity map.
            if not pkg.metadata_targets:
                try:
                    manifest = tomllib.loads(pkg.manifest_path.read_text(encoding="utf-8"))
                    for alias, dep in manifest.get("dependencies", {}).items():
                        if isinstance(dep, dict) and dep.get("workspace"):
                            continue  # no assumed inherited identity in synthetic mode
                        alias_norm = _norm_crate_name(alias)
                        if alias_norm not in {"std", "core"}:
                            self.externals[alias_norm] = dep.get("package", alias) if isinstance(dep, dict) else alias
                except (OSError, UnicodeError, tomllib.TOMLDecodeError):
                    pass
        for scope, blocks, i, kind, value in self.events:
            location = (scope, blocks)
            if kind == "use":
                for path, alias in value:
                    self.imports.append((location, alias, path))
            elif kind == "macro_definition":
                self.macros.append((location, value))
            elif kind == "module":
                self.shadows.append((location, value))
            elif kind == "token" and value.kind == "IDENT" and value.value in {
                "mod", "struct", "enum", "type", "trait", "union", "fn", "let", "const", "static", "extern"
            }:
                j = i + 1
                if j < len(tokens) and tokens[j].value in {"mut", "crate"}:
                    j += 1
                if j < len(tokens) and tokens[j].kind == "IDENT":
                    self.shadows.append((location, tokens[j].value))
                    if value.value == "extern" and j+2 < len(tokens) and tokens[j+1].value == "as":
                        self.shadows.append((location, tokens[j+2].value))

    @staticmethod
    def _visible(declared, at):
        scope, blocks = declared
        return scope == at[0] and at[1][:len(blocks)] == blocks

    def at(self, i):
        return self.locations.get(i, ((), ()))

    def identity(self, path, i, *, macro=False):
        """Resolve a supported external/builtin path at one occurrence."""
        return self._identity(path, self.at(i), macro, frozenset())

    def _identity(self, path, at, macro, seen):
        if not path:
            return None
        absolute = path[0] == "::"
        if absolute:
            path = path[1:]
        if not path or path[0] in {"crate", "self", "super"}:
            return None
        first = path[0]
        if not absolute:
            if any(n == first and self._visible(loc, at) for loc, n in self.shadows):
                return None
            if macro and any(n == first and at[0][:len(loc[0])] == loc[0]
                             and at[1][:len(loc[1])] == loc[1] for loc, n in self.macros):
                return None
            matches = [(loc, p) for loc, alias, p in self.imports
                       if alias == first and self._visible(loc, at)]
            if matches:
                depth = max(len(loc[1]) for loc, _ in matches)
                nearest = [(loc, p) for loc, p in matches if len(loc[1]) == depth]
                if len(nearest) != 1:
                    return None
                loc, imported = nearest[0]
                if (loc, first) in seen:
                    return None
                # A bare `use std;` or `use bevy;` imports the external root
                # itself; a longer self-referential alias is not proof.
                if imported == (first,) and first in self.externals:
                    return (self.externals[first],) + path[1:]
                identity = self._identity(imported, loc, macro, seen | {(loc, first)})
                return identity + path[1:] if identity is not None else None
            if macro and len(path) == 1 and first in _BUILTIN_MACROS:
                if any(alias is None and self._visible(loc, at) for loc, alias, _ in self.imports):
                    return None
                return ("builtin", first)
        root = self.externals.get(first)
        return (root,) + path[1:] if root is not None else None

    def macro_name(self, path, i):
        identity = self.identity(path, i, macro=True)
        if identity is None:
            return None
        if len(identity) == 2 and identity[0] in {"builtin", "std", "core"} and identity[1] in _BUILTIN_MACROS:
            return identity[1]
        if identity[:2] == ("bevy", "asset") and len(identity) == 3 and identity[2] in {"load_internal_asset", "embedded_asset"}:
            return identity[2]
        return None


def _tokens_use_asset_macro(tokens: list[Token], consumer_abs: Path,
                            candidate_abs: Path, root_real: Path, pkg: PackageInfo | None) -> bool:
    context = _ProofContext(tokens, pkg)
    for i, path, args in _macro_invocations(tokens):
        name = context.macro_name(path, i)
        if name == "load_internal_asset" and len(args) == 4:
            arg = args[2]
        elif name == "embedded_asset" and len(args) == 2:
            arg = args[1]
        elif name in {"include_str", "include_bytes"} and len(args) == 1:
            arg = args[0]
        else:
            continue
        if _path_expression(arg, consumer_abs.parent, None, {}, context=context, at=i) == candidate_abs:
            return True
    return False


def _use_leaves(tokens: list[Token]) -> list[tuple[tuple[str, ...], str | None]]:
    """Flatten supported Rust use trees, retaining complete paths and alias names."""
    i = 0
    result: list[tuple[tuple[str, ...], str | None]] = []
    def tree(prefix: tuple[str, ...]):
        nonlocal i
        path = prefix
        if i < len(tokens) and tokens[i].value == "::":
            path += ("::",)
            i += 1
        while i < len(tokens):
            t = tokens[i]
            if t.value == "{":
                i += 1
                while i < len(tokens) and tokens[i].value != "}":
                    tree(path)
                    if i < len(tokens) and tokens[i].value == ",":
                        i += 1
                    elif i >= len(tokens) or tokens[i].value != "}":
                        raise ValueError("unsupported use tree")
                if i >= len(tokens):
                    raise ValueError("unterminated use tree")
                i += 1
                return
            if t.kind != "IDENT" and t.value != "*":
                raise ValueError("unsupported use leaf")
            path += (t.value,)
            i += 1
            if i < len(tokens) and tokens[i].value == "::":
                i += 1
                continue
            alias = path[-1]
            if i < len(tokens) and tokens[i].value == "as":
                i += 1
                if i >= len(tokens) or tokens[i].kind != "IDENT":
                    raise ValueError("invalid alias")
                alias = tokens[i].value
                i += 1
            if path[-1] == "self" and len(path) > 1:
                if alias == "self":
                    alias = path[-2]
                path = path[:-1]
            result.append((path, alias if alias != "*" else None))
            return
        raise ValueError("empty use tree")
    try:
        tree(())
        return result if i == len(tokens) else []
    except ValueError:
        return []


def _scope_tokens(tokens: list[Token]):
    """Index inline module scopes and lexical block paths. Ignore macro token trees."""
    scope: tuple[str, ...] = ()
    blocks: list[tuple[tuple[str, ...], int]] = []
    i = 0
    while i < len(tokens):
        t = tokens[i]
        v = _values(tokens[i:i+3])
        if t.kind == "PUNCT" and t.value == "#":
            opening = i + 1 + (i+1 < len(tokens) and tokens[i+1].value == "!")
            if opening < len(tokens) and tokens[opening].value == "[":
                args, end = _call_args(tokens, opening)
                if args is not None:
                    yield scope, tuple(b[1] for b in blocks), i, "token", t
                    i = end
                    continue
        if t.kind == "IDENT" and t.value == "macro_rules" and i+3 < len(tokens) and tokens[i+1].value == "!":
            _, end = _call_args(tokens, i+3)
            yield scope, tuple(b[1] for b in blocks), i, "macro_definition", tokens[i+2].value
            i = max(i+1, end)
            continue
        if t.kind == "IDENT" and len(v) == 3 and v[0] == "mod" and tokens[i+1].kind == "IDENT" and v[2] == "{":
            yield scope, tuple(b[1] for b in blocks), i, "module", v[1]
            blocks.append((scope, i))
            scope += (v[1],)
            i += 3
            continue
        # Preserve invocation identity and location, but never promote tokens
        # inside the tree into real lexical declarations or executable calls.
        if t.kind == "IDENT" or t.value == "::":
            j = i
            path = []
            if t.value == "::":
                path.append("::")
                j += 1
            if j < len(tokens) and tokens[j].kind == "IDENT":
                path.append(tokens[j].value)
                j += 1
                while j+1 < len(tokens) and tokens[j].value == "::" and tokens[j+1].kind == "IDENT":
                    path.append(tokens[j+1].value)
                    j += 2
                if j+1 < len(tokens) and tokens[j].value == "!":
                    args, end = _call_args(tokens, j+1)
                    if args is not None:
                        yield scope, tuple(b[1] for b in blocks), i, "macro", (tuple(path), args)
                        i = end
                        continue
        if t.value == "use" and t.kind == "IDENT":
            j = i+1
            while j < len(tokens) and tokens[j].value != ";":
                j += 1
            yield scope, tuple(b[1] for b in blocks), i, "use", _use_leaves(tokens[i+1:j])
            i = j+1
            continue
        if t.kind == "PUNCT" and t.value == "{":
            blocks.append((scope, i))
        elif t.kind == "PUNCT" and t.value == "}" and blocks:
            scope, _ = blocks.pop()
        yield scope, tuple(b[1] for b in blocks), i, "token", t
        i += 1


class _ModuleIndex:
    """Resolve fixture references in each actual crate/module context, not by suffix.

    This is a bounded source model, not rustc: ambiguous bindings, glob-only imports,
    macro-generated paths and unsupported constructs do not constitute proof.
    """
    def __init__(self, root, packages, files, by_target):
        self.root, self.files = root, files
        self.events = {f: list(_scope_tokens(ts)) for f, ts in files.items()}
        children: dict[str, list[ModBinding]] = {}
        for group in by_target.values():
            for binding in group:
                children.setdefault(binding.owner_rel, []).append(binding)
        self.contexts = []
        for pkg in packages:
            if pkg.is_support:
                continue
            roots = pkg.prod_target_files or [pkg.dir_path / "src" / name for name in ("lib.rs", "main.rs")]
            for entry in roots:
                if not _is_within(entry, root):
                    continue
                rel = _to_posix_rel(entry, root)
                if rel not in files:
                    continue
                graph: dict[tuple[str, ...], set[str | None]] = {}
                locations: dict[str, set[tuple[str, ...]]] = {}
                mod_imports: dict[tuple[str, ...], dict[str, list[tuple[tuple[str, ...], bool]]]] = {}
                mod_shadows: dict[tuple[str, ...], set[str]] = {}
                def visit(file, prefix, ancestors):
                    if file in ancestors or file not in files:
                        return
                    graph.setdefault(prefix, set()).add(file)
                    locations.setdefault(file, set()).add(prefix)
                    ftokens = self.files[file]
                    for scope, blocks, i, kind, value in self.events[file]:
                        if kind == "module" and len(blocks) == len(scope):
                            graph.setdefault(prefix + scope + (value,), set()).add(None)
                        elif len(blocks) == len(scope):
                            mod_key = prefix + scope
                            if kind == "use":
                                is_pub = i > 0 and ftokens[i - 1].kind == "IDENT" and ftokens[i - 1].value == "pub"
                                for upath, alias in value:
                                    if alias:
                                        mod_imports.setdefault(mod_key, {}).setdefault(alias, []).append((upath, is_pub))
                            elif kind == "token" and value.kind == "IDENT" and value.value in {
                                "struct", "enum", "type", "trait", "union", "fn", "let", "const", "static"
                            }:
                                if i + 1 < len(ftokens) and ftokens[i + 1].kind == "IDENT":
                                    mod_shadows.setdefault(mod_key, set()).add(ftokens[i + 1].value)
                    for b in children.get(file, []):
                        if not _is_within(b.resolved_target, root):
                            continue
                        target = _to_posix_rel(b.resolved_target, root)
                        visit(target, prefix + b.scope + (b.local_name,), ancestors | {file})
                visit(rel, (), set())
                self.contexts.append((graph, locations, mod_imports, mod_shadows))

    def reachable(self, file: str) -> bool:
        return any(file in locations for _, locations, _, _ in self.contexts)

    def attachment_owner_reachable(self, attachment: AttachmentRecord) -> bool:
        suffix = tuple(attachment.module_path.split("::")) if attachment.module_path else ()
        return any(graph.get(prefix) == {attachment.owner_file}
                   and (not suffix or graph.get(prefix + suffix) == {None})
                   for graph, locations, _, _ in self.contexts
                   for prefix in locations.get(attachment.owner_file, ()))

    def consumes(self, consumer: str, expected: Path, owners: list[ModBinding]) -> bool:
        target = _to_posix_rel(expected, self.root)
        tokens = self.files.get(consumer, [])
        events = self.events.get(consumer, [])
        for graph, locations, mod_imports, mod_shadows in self.contexts:
            # Require the exact owner declaration to be reachable in this crate target.
            targets = {p + b.scope + (b.local_name,) for b in owners
                       for p in locations.get(b.owner_rel, ())
                       if graph.get(p + b.scope + (b.local_name,)) == {target}}
            if not targets:
                continue
            for prefix in locations.get(consumer, ()):
                imports: dict[tuple, dict[str, list[tuple[str, ...]]]] = {}
                shadows: dict[tuple, set[str]] = {}
                potential_binders: set[str] = set()
                for scope, blocks, i, kind, value in events:
                    key = (scope, blocks)
                    if kind == "use":
                        for path, alias in value:
                            if alias:
                                imports.setdefault(key, {}).setdefault(alias, []).append(path)
                    elif kind == "token" and value.kind == "IDENT" and value.value in {"struct", "enum", "type", "trait", "union", "fn", "let", "const", "static"}:
                        if i+1 < len(tokens) and tokens[i+1].kind == "IDENT":
                            shadows.setdefault(key, set()).add(tokens[i+1].value)
                    if kind != "token" or value.kind != "IDENT":
                        continue
                    if i+1 < len(tokens) and tokens[i+1].value == ":":
                        # Function parameters, typed patterns and struct fields.
                        potential_binders.add(value.value)
                    if value.value in {"let", "for"}:
                        j = i + 1
                        while j < len(tokens) and tokens[j].value not in {"=", ";", "in", "{", ":"}:
                            if tokens[j].kind == "IDENT":
                                potential_binders.add(tokens[j].value)
                            j += 1
                # Closure/match pattern bindings are deliberately conservative:
                # no unqualified reference can establish provenance when its name
                # is also a plausible pattern binding in this consumer file.
                for j, token in enumerate(tokens):
                    if token.kind != "PUNCT":
                        continue
                    if token.value == "=>":
                        k = j - 1
                        while k >= 0 and tokens[k].value not in {";", "{", "}", "=>", ",", "|"}:
                            if tokens[k].kind == "IDENT":
                                potential_binders.add(tokens[k].value)
                            k -= 1
                    elif token.value == "|" and j and tokens[j-1].value in {"=", "(", ",", "move", "return"}:
                        k = j + 1
                        while k < len(tokens) and tokens[k].value not in {"|", ";", "{"}:
                            if tokens[k].kind == "IDENT" and (k+1 == len(tokens) or tokens[k+1].value in {":", ",", "|"}):
                                potential_binders.add(tokens[k].value)
                            k += 1

                def resolve_anchor(path, anchor_mod):
                    if not path or path[0] == "::":
                        return None
                    first, rest = path[0], path[1:]
                    if first == "crate":
                        return rest
                    if first == "self":
                        return anchor_mod + rest
                    if first == "super":
                        count = 0
                        while count < len(path) and path[count] == "super":
                            count += 1
                        if count > len(anchor_mod):
                            return None
                        return anchor_mod[:-count] + path[count:]
                    return anchor_mod + path

                def resolve_absolute(absolute, consumer_mod, mod_seen=frozenset()):
                    # Match only a prefix corresponding to the exact target module;
                    # require at least one member beyond that module.
                    for n in range(len(absolute) - 1, 0, -1):
                        candidate = absolute[:n]
                        if candidate in graph:
                            if candidate in targets:
                                return candidate
                            member = absolute[n]
                            if member not in mod_shadows.get(candidate, set()):
                                is_ancestor = len(consumer_mod) >= len(candidate) and consumer_mod[:len(candidate)] == candidate
                                bindings = [
                                    upath for upath, is_pub in mod_imports.get(candidate, {}).get(member, [])
                                    if is_ancestor or is_pub
                                ]
                                if len(bindings) == 1 and (candidate, member) not in mod_seen:
                                    return resolve_in_mod(
                                        bindings[0] + absolute[n + 1:],
                                        candidate,
                                        consumer_mod,
                                        mod_seen | {(candidate, member)},
                                    )
                            return None
                    return None

                def resolve_in_mod(path, ctx_mod, consumer_mod, mod_seen=frozenset()):
                    if not path or path[0] == "::":
                        return None
                    first = path[0]
                    if first not in {"crate", "self", "super"}:
                        if first in mod_shadows.get(ctx_mod, set()):
                            return None
                        is_ancestor = len(consumer_mod) >= len(ctx_mod) and consumer_mod[:len(ctx_mod)] == ctx_mod
                        bindings = [
                            upath for upath, is_pub in mod_imports.get(ctx_mod, {}).get(first, [])
                            if is_ancestor or is_pub
                        ]
                        if bindings:
                            if len(bindings) != 1 or (ctx_mod, first) in mod_seen or bindings[0] == (first,):
                                return None
                            return resolve_in_mod(
                                bindings[0] + path[1:],
                                ctx_mod,
                                consumer_mod,
                                mod_seen | {(ctx_mod, first)},
                            )
                    absolute = resolve_anchor(path, ctx_mod)
                    return resolve_absolute(absolute, consumer_mod, mod_seen) if absolute else None

                def resolve(path, scope, blocks, seen=frozenset()):
                    if not path or path[0] == "::":
                        return None
                    module = prefix + scope
                    first = path[0]
                    if first in {"crate", "self", "super"}:
                        absolute = resolve_anchor(path, module)
                        return resolve_absolute(absolute, module) if absolute else None
                    if first in potential_binders:
                        return None
                    # Most-local explicit import wins; block-scoped aliases do not leak.
                    for depth in range(len(blocks), -1, -1):
                        key = (scope, blocks[:depth])
                        if first in shadows.get(key, set()):
                            return None
                        bindings = imports.get(key, {}).get(first, [])
                        if bindings:
                            if len(bindings) != 1 or key + (first,) in seen:
                                return None
                            bound = bindings[0]
                            # `use fixtures;` can refer to the local module itself.
                            if bound == (first,):
                                break
                            return resolve(bound + path[1:], scope, blocks[:depth], seen | {key + (first,)})
                    return resolve_absolute(module + path, module)
                for scope, blocks, i, kind, value in events:
                    if kind == "token" and value.kind == "IDENT":
                        if i and tokens[i-1].value in ("::", ".", "fn", "let", "struct", "enum", "trait", "type", "mod", "const", "static"):
                            continue
                        path = [value.value]
                        j = i+1
                        while j+1 < len(tokens) and tokens[j].value == "::" and tokens[j+1].kind == "IDENT":
                            path.append(tokens[j+1].value)
                            j += 2
                        if len(path) > 1 and resolve(tuple(path), scope, blocks) in targets:
                            return True
                        if len(path) == 1:
                            # A used member alias (not its declaration) is also valid.
                            if resolve(tuple(path), scope, blocks) in targets:
                                return True
        return False


def _package_for(path: Path, packages: list[PackageInfo]) -> PackageInfo | None:
    matches = [p for p in packages if _is_within(path, p.dir_path)]
    return max(matches, key=lambda p: len(p.dir_path.parts), default=None)


def _out_key(pkg: PackageInfo, rel: str | None) -> str | None:
    rel = _relative(rel or "")
    return f"OUT_DIR:{pkg.rel_dir}/{rel}" if rel else None


def _function_body(tokens: list[Token], start: int) -> tuple[int, int] | None:
    if start + 2 >= len(tokens) or tokens[start].value != "fn" or tokens[start+1].kind != "IDENT":
        return None
    opening = start + 2
    while opening < len(tokens) and tokens[opening].value not in {"(", ";", "{"}:
        opening += 1
    args, end = _call_args(tokens, opening)
    if args is None:
        return None
    while end < len(tokens) and tokens[end].value not in {"{", ";"}:
        end += 1
    if end == len(tokens) or tokens[end].value != "{":
        return None
    _, stop = _call_args(tokens, end)
    return (end, stop) if stop > end + 1 else None


def _deferred_execution_ranges(tokens, scope_at):
    """Recognize closure/async bodies; unknown expression-body extent is conservative."""
    ranges = []
    for i in scope_at:
        value = tokens[i].value
        if value == "async" and i+1 < len(tokens):
            j = i+1 + (tokens[i+1].value == "move")
            if j < len(tokens) and tokens[j].value == "{":
                _, end = _call_args(tokens, j)
                ranges.append((j, end))
        if value != "|" or not i or tokens[i-1].value not in {"=", "(", ",", "move", "return", "{", "=>", ":"}:
            continue
        j = i+1
        while j < len(tokens) and tokens[j].value not in {"|", ";", "{"}:
            j += 1
        if j >= len(tokens) or tokens[j].value != "|":
            continue
        start = j+1
        if start < len(tokens) and tokens[start].value == "{":
            _, end = _call_args(tokens, start)
        else:
            end = start
            # Skip balanced expression groups, stop at the statement boundary.
            while end < len(tokens) and tokens[end].value not in {";", ",", ")", "}"}:
                if tokens[end].value in {"(", "[", "{"}:
                    _, end = _call_args(tokens, end)
                else:
                    end += 1
        ranges.append((start, end))
    return ranges


def _generation_refs(tokens: list[Token], origin: Path, root: Path, pkg: PackageInfo,
                     *, producer: bool, allow_asset_resolver: bool = False) -> set[Path | str]:
    """Recognize exact supported operation arguments and immutable local path bindings.

    Only the observed fs::write and explicit read/include/asset resolver contracts
    are supported; a log, comment, arbitrary call or basename is never proof.
    """
    # Record declaration scope and position; only a visible, earlier immutable
    # binding can prove a write/read argument. Reject mutated or ambiguous names.
    context = _ProofContext(tokens, pkg)
    events = context.events
    scope_at = {i: (scope, blocks) for scope, blocks, i, kind, _ in events if kind == "token"}
    main_bodies = [body for i, (scope, blocks) in scope_at.items()
                   if not scope and not blocks and tokens[i].value == "fn"
                   and i+1 < len(tokens) and tokens[i+1].value == "main"
                   and (body := _function_body(tokens, i)) is not None]
    # Nearest function ownership, not merely outer main's numerical range.
    function_bodies = [body for i in scope_at if tokens[i].value == "fn"
                       and (body := _function_body(tokens, i)) is not None]
    # Closures are outside the direct-main contract. Track common closure
    # prefixes, including move and zero-argument forms; expression bodies end
    # at the enclosing statement/argument boundary. Async blocks are excluded too.
    deferred = _deferred_execution_ranges(tokens, scope_at)
    def directly_in_main(i):
        owners = [body for body in function_bodies if body[0] < i < body[1]]
        return (len(main_bodies) == 1 and owners and max(owners, key=lambda b: b[0]) == main_bodies[0]
                and not any(start <= i < end for start, end in deferred))

    declarations: list[tuple[int, tuple, tuple, str, Path | str | None]] = []
    joinable_declarations: set[tuple[int, str]] = set()
    def visible(i: int) -> dict[str, Path | str]:
        scope, blocks = scope_at.get(i, ((), ()))
        result: dict[str, Path | str] = {}
        for at, ds, db, name, val in declarations:
            if at >= i or ds != scope or blocks[:len(db)] != db:
                continue
            if val is None:
                result.pop(name, None)
            else:
                result[name] = val
        return result

    def visible_joinable(i):
        scope, blocks = scope_at.get(i, ((), ()))
        result = set()
        for at, ds, db, name, val in declarations:
            if at >= i or ds != scope or blocks[:len(db)] != db:
                continue
            if val is not None and (at, name) in joinable_declarations:
                result.add(name)
            else:
                result.discard(name)
        return frozenset(result)

    for i, t in enumerate(tokens):
        if i in scope_at and t.kind == "IDENT" and t.value == "for":
            end = i+1
            while end < len(tokens) and tokens[end].value not in {"in", "{", ";"}:
                end += 1
            if end < len(tokens) and tokens[end].value == "in":
                scope, blocks = scope_at[i]
                for name_token in tokens[i+1:end]:
                    if name_token.kind == "IDENT":
                        declarations.append((i, scope, blocks, name_token.value, None))
        if i not in scope_at or t.value != "let" or i+3 >= len(tokens):
            continue
        j = i+1
        mutable = tokens[j].value == "mut"
        if mutable:
            j += 1
        if tokens[j].kind != "IDENT" or tokens[j+1].value != "=":
            # Unknown typed/destructured binding may shadow a previously proven
            # name; invalidate names in its pattern rather than reuse stale paths.
            end = j
            while end < len(tokens) and tokens[end].value not in {"=", ";"}:
                end += 1
            scope, blocks = scope_at[i]
            for t in tokens[j:end]:
                if t.kind == "IDENT":
                    declarations.append((i, scope, blocks, t.value, None))
            continue
        name = tokens[j].value
        end = j+2
        while end < len(tokens) and tokens[end].value != ";":
            end += 1
        expr = tokens[j+2:end]
        joinable = visible_joinable(i)
        val = None if mutable else _path_expression(expr, pkg.dir_path, pkg.dir_path, visible(i), context=context, at=i, joinable=joinable)
        # A literal/concat expression yields a string, not a Path receiver.
        # A successful recognized constructor or existing proven Path binding
        # preserves the receiver type through immutable aliases and joins.
        values = _values(expr)
        while values[:1] == ["&"]:
            values = values[1:]
        if val is not None and values:
            opening = values.index("(") if "(" in values else -1
            head = tuple(x for x in values[:opening] if x != "::") if opening >= 0 else ()
            if values[:1] == ["::"]:
                head = ("::",) + head
            if values[0] in joinable or context.identity(head, i) in {("std", "path", "PathBuf", "from"), ("std", "path", "Path", "new")}:
                joinable_declarations.add((i, name))
        scope, blocks = scope_at[i]
        declarations.append((i, scope, blocks, name, val))
    # Reject mutation of a path binding even through a later nested block.
    for i, t in enumerate(tokens[:-1]):
        if i not in scope_at or t.kind != "IDENT" or tokens[i+1].value != "=" or (i and tokens[i-1].value in ("let", "mut")):
            continue
        scope, blocks = scope_at[i]
        declarations = [(at, ds, db, name, None if name == t.value and ds == scope and blocks[:len(db)] == db else val)
                        for at, ds, db, name, val in declarations]
    # Source macros are only accepted if they are actual invocations, not nested
    # inside arbitrary token trees (e.g. stringify! or macro_rules! definitions).
    macro_calls = list(_macro_invocations(tokens))
    refs: set[Path | str] = set()
    if not producer:
        for i, macro_path, args in macro_calls:
            name = context.macro_name(macro_path, i)
            if name in {"include", "include_bytes", "include_str"} and len(args) == 1:
                val = _path_expression(args[0], origin.parent, pkg.dir_path, {}, context=context, at=i)
                if isinstance(val, Path):
                    refs.add(val)
                elif isinstance(val, str) and val.startswith("OUT_DIR:"):
                    key = _out_key(pkg, val[8:])
                    if key:
                        refs.add(key)
    for i, t in enumerate(tokens):
        if t.kind != "IDENT" or i not in scope_at:
            continue
        if i and tokens[i-1].value in ("::", "."):
            continue
        j = i+1
        while j+1 < len(tokens) and tokens[j].value == "::" and tokens[j+1].kind == "IDENT":
            j += 2
        path = tuple(_values(tokens[i:j]))
        if j >= len(tokens):
            continue
        is_macro = tokens[j].value == "!"
        if is_macro:
            continue
        bindings = visible(i)
        args, _ = _call_args(tokens, j)
        if not args:
            continue
        val = None
        identity = context.identity(tuple(x for x in path if x != "::"), i)
        # Dispatch on the resolved operation, never the local alias spelling.
        # Different fs members assign different semantic roles to arguments.
        if producer and identity == ("std", "fs", "write") and len(args) == 2:
            # A dormant helper or a macro definition is not a verified producer.
            # The supported build-script contract writes directly from main.
            if not directly_in_main(i):
                continue
            val = _path_expression(args[0], pkg.dir_path, pkg.dir_path, bindings, context=context, at=i, joinable=visible_joinable(i))
        elif (not producer and identity in {("std", "fs", "read"), ("std", "fs", "read_to_string")}
              and len(args) == 1):
            val = _path_expression(args[0], pkg.dir_path, pkg.dir_path, bindings, context=context, at=i, joinable=visible_joinable(i))
        elif (not producer and allow_asset_resolver
              and path == ("usd_support", "::", "resolve_asset_path")
              and not any(n == "usd_support" and context._visible(loc, context.at(i))
                          for loc, n in context.shadows)
              and not any(alias == "usd_support" and context._visible(loc, context.at(i))
                          for loc, alias, _ in context.imports)
              and len(args) == 1):
            rel = _relative(_literal(args[0]) or "")
            if rel:
                val = Path(os.path.realpath(root / "assets" / rel))
        if isinstance(val, str) and val.startswith("OUT_DIR:"):
            key = _out_key(pkg, val[8:])
            if key:
                refs.add(key)
        elif isinstance(val, Path):
            refs.add(val)
    return refs


def _has_reviewed_usd_asset_resolver_contract(path: Path) -> bool:
    """Check the exact currently supported implementation contract, not its name.

    This narrow repository-owned path resolver is the only nonstandard consumer
    API accepted by the generated-source guard. A changed implementation needs
    explicit review/support; comments and formatting do not affect this check.
    """
    try:
        tokens = _tokenize_rust(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError):
        return False
    if any(t.kind == "LEX_ERROR" for t in tokens):
        return False
    expected = {
        "workspace_root": '''
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent().and_then(Path::parent).and_then(Path::parent)
                .expect("usd_support crate is beneath tests/support/usd")
        ''',
        "validate_relative_path": '''
            let path = Path::new(relative);
            assert!(
                !path.as_os_str().is_empty()
                    && path.components().all(|part| matches!(part, Component::Normal(_))),
                "{domain} must be a relative path with normal components only: {}",
                path.display()
            );
            path
        ''',
        "resolve_asset_path": '''
            let relative = validate_relative_path(relative, "asset");
            let path = workspace_root().join("assets").join(relative);
            assert!(path.is_file(), "asset file does not exist: {}", path.display());
            path
        ''',
    }
    context = _ProofContext(tokens)
    found: dict[str, list[list[tuple[str, str]]]] = {}
    for scope, blocks, i, kind, tok in context.events:
        if scope or blocks or kind != "token" or tok.value != "fn" or i+1 >= len(tokens):
            continue
        name = tokens[i+1].value
        if name not in expected:
            continue
        if any(context.identity((typ,), i) != ("std", "path", typ)
               for typ in ("Path", "PathBuf", "Component")) or context.macro_name(("env",), i) != "env":
            return False
        body = _function_body(tokens, i)
        if body is None:
            return False
        opening, end = body
        found.setdefault(name, []).append([(t.kind, t.value) for t in tokens[opening+1:end-1]])
    return all(found.get(name) == [[(t.kind, t.value) for t in _tokenize_rust(body)]]
               for name, body in expected.items())


def _uses_workspace_usd_asset_resolver(pkg: PackageInfo, packages: list[PackageInfo], root: Path) -> bool:
    # The repository-owned resolver contract is workspace_root()/assets/<normal relative path>.
    # Only the canonical support package and its explicit dev-dependency consumers
    # can use this contract; a same-named module or unrelated package is not proof.
    support = next((p for p in packages if p.name == "usd_support" and p.is_support
                    and p.dir_path == root / "tests/support/usd"), None)
    if support is None or not _has_reviewed_usd_asset_resolver_contract(support.dir_path / "lib.rs"):
        return False
    if pkg.metadata_deps:
        return any(dep.get("name") == support.name and dep.get("kind") == "dev"
                   and _norm_crate_name(dep.get("rename") or support.name) == "usd_support"
                   and dep.get("path") is not None
                   and Path(os.path.realpath(dep["path"])) == support.dir_path
                   for dep in pkg.metadata_deps)
    try:
        manifest = tomllib.loads(pkg.manifest_path.read_text(encoding="utf-8"))
        ws = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
        dep = manifest.get("dev-dependencies", {}).get("usd_support")
        if not isinstance(dep, dict):
            return False
        base = pkg.dir_path
        if dep.get("workspace") is True:
            dep = ws.get("workspace", {}).get("dependencies", {}).get("usd_support")
            base = root
        return isinstance(dep, dict) and isinstance(dep.get("path"), str) and Path(os.path.realpath(base / dep["path"])) == support.dir_path
    except (OSError, UnicodeError, tomllib.TOMLDecodeError):
        return False


def _verify_generated(entries, root, packages, src_roots, prod_tokens, module_index, violations):
    verified = {}
    in_src = set()
    seen = set()
    for entry in entries:
        key = entry["path"]
        if key in seen:
            violations.append(Violation("duplicate-generated-source", key, 1, "duplicate generated classification"))
            continue
        seen.add(key)
        producer = Path(os.path.realpath(root / entry["producer"]))
        pkg = _package_for(producer, packages)
        target = key if key.startswith("OUT_DIR:") else Path(os.path.realpath(root / key))
        valid = True
        if isinstance(target, Path):
            if not _is_within(target, root) or not target.is_file() or _to_posix_rel(target, root) != key:
                violations.append(Violation("missing-generated-source", key, 1, "missing or noncanonical generated target"))
                valid = False
        elif pkg is None or not key.startswith(f"OUT_DIR:{pkg.rel_dir}/") or _out_key(pkg, key[len(f'OUT_DIR:{pkg.rel_dir}/'):]) != key:
            violations.append(Violation("invalid-generated-target", key, 1, "invalid package-scoped OUT_DIR target"))
            valid = False
        if pkg is None or not _is_within(producer, root) or not producer.is_file() or producer != pkg.dir_path / "build.rs":
            violations.append(Violation("missing-generated-producer", key, 1, "expected actual package build.rs producer"))
            valid = False
        else:
            text = _read_text_strict(producer, entry["producer"], violations)
            ts = _tokenize_rust(text) if text is not None else []
            if any(t.kind == "LEX_ERROR" for t in ts) or target not in _generation_refs(ts, producer, root, pkg, producer=True):
                violations.append(Violation("unverified-generated-producer", key, 1, "no supported write operation resolves to exact generated target"))
                valid = False
        production_consumer = False
        for cons in entry["consumers"]:
            path = Path(os.path.realpath(root / cons))
            cpkg = _package_for(path, packages)
            if cpkg is None or not _is_within(path, root) or not path.is_file() or path == producer:
                violations.append(Violation("missing-generated-consumer", key, 1, f"invalid independent consumer {cons}"))
                valid = False
                continue
            ts = prod_tokens.get(cons)
            if ts is None:
                text = _read_text_strict(path, cons, violations)
                ts = _tokenize_rust(text) if text is not None else []
            if any(t.kind == "LEX_ERROR" for t in ts) or target not in _generation_refs(
                    ts, path, root, cpkg, producer=False,
                    allow_asset_resolver=_uses_workspace_usd_asset_resolver(cpkg, packages, root)):
                violations.append(Violation("unverified-generated-consumer", key, 1, f"no supported consumer operation resolves to exact generated target: {cons}"))
                valid = False
            elif cons in prod_tokens and module_index.reachable(cons):
                production_consumer = True
        if isinstance(target, Path) and any(_is_within(target, sr) for sr in src_roots) and not production_consumer:
            violations.append(Violation("test-only-generated-fixture-in-src", key, 1, "generated source inside src requires a verified production consumer"))
            valid = False
        if valid:
            verified[key] = entry
            if isinstance(target, Path) and any(_is_within(target, sr) for sr in src_roots):
                in_src.add(key)
    return verified, in_src


def _has_test_attribute(path: Path, root: Path, suffixes: set[str], violations: list[Violation]) -> bool:
    text = _read_text_strict(path, _to_posix_rel(path, root), violations)
    if text is None:
        return False
    tokens = _tokenize_rust(text)
    if any(t.kind == "LEX_ERROR" for t in tokens):
        violations.append(Violation("unsupported-test-source-syntax", _to_posix_rel(path, root), 1, "cannot census test attributes"))
        return False
    def is_test(node: AttrNode) -> bool:
        if node.segments and node.segments[-1] in suffixes:
            return True
        return node.segments == ("cfg_attr",) and any(is_test(child) for child in node.args[1:])
    actual = {i for scope, blocks, i, kind, _ in _scope_tokens(tokens)
              if kind == "token" and len(blocks) == len(scope)}
    for i, t in enumerate(tokens[:-2]):
        if i not in actual or t.kind != "PUNCT" or t.value != "#" or tokens[i+1].value != "[":
            continue
        args, end = _call_args(tokens, i+1)
        if args is None or not args or not args[0]:
            continue
        node, _ = _parse_attr_meta_item(tokens, i+2, end-1)
        if not is_test(node):
            continue
        # Skip subsequent attributes and Rust function qualifiers only. A
        # commented/stringified test attribute or one on a non-fn is not a test.
        j = end
        while j < len(tokens):
            if tokens[j].value == "#" and j+1 < len(tokens) and tokens[j+1].value == "[":
                _, j = _call_args(tokens, j+1)
            elif tokens[j].value in {"async", "unsafe", "const", "pub"}:
                j += 1
                if j < len(tokens) and tokens[j].value == "(":
                    _, j = _call_args(tokens, j)
            else:
                break
        if j+1 < len(tokens) and tokens[j].kind == "IDENT" and tokens[j].value == "fn" and tokens[j+1].kind == "IDENT":
            return True
    return False


def evaluate_workspace(
    root: Path,
    policy_path: Path | None = None,
    *,
    skip_cargo_metadata: bool = False,
) -> dict[str, Any]:
    root_real = Path(os.path.realpath(root))
    resolved_policy = policy_path if policy_path is not None else (root_real / "scripts" / "test_architecture_policy.json")
    violations: list[Violation] = []

    policy = _validate_policy(resolved_policy, root_real, violations)
    if policy is None:
        return {
            "status": "fail",
            "root": str(root_real),
            "cargo_metadata_used": False,
            "workspace_members": [],
            "src_roots": [],
            "test_support_crates": [],
            "reviewed_path_only_attachments_count": 0,
            "test_module_count": 0,
            "tests_root_count": 0,
            "generated_sources_count": 0,
            "exceptional_production_fixtures_count": 0,
            "violation_count": len(violations),
            "violations": [v.to_dict() for v in violations],
        }

    packages, ws_dep_pkg_map, cargo_metadata_used = _discover_workspace(
        root_real, policy, skip_cargo_metadata, violations
    )
    support_norm_names, support_aliases_by_pkg = _audit_dependencies(
        packages, ws_dep_pkg_map, root_real, cargo_metadata_used, violations
    )

    forbidden_file_names = set(policy["forbidden_test_file_names"])
    forbidden_suffixes = set(policy["forbidden_test_attr_suffixes"])

    all_src_roots = sorted({sr for p in packages for sr in p.src_roots})
    all_src_roots_rel = [_to_posix_rel(sr, root_real) for sr in all_src_roots if _is_within(sr, root_real)]

    observed_attachments: list[AttachmentRecord] = []
    mod_bindings_by_target: dict[Path, list[ModBinding]] = {}
    prod_tokens_by_file: dict[str, list[Token]] = {}
    include_markers: list[IncludeMarker] = []
    scanned_src_files: set[Path] = set()
    candidate_fixtures_in_src: set[str] = set()

    for pkg in packages:
        # Support crates with a src/ dir were already flagged in _audit_dependencies,
        # AND we still scan every src root unconditionally here (no silent skip).
        pkg_forbidden_aliases = set(support_norm_names) | support_aliases_by_pkg.get(pkg.name, set())

        pkg_dirs: set[Path] = set()
        pkg_files: set[Path] = set()
        for sr in pkg.src_roots:
            if sr.name != "src" and sr == pkg.dir_path:
                for pt in pkg.prod_target_files:
                    if pt.is_file():
                        pkg_files.add(pt)
                continue
            if sr.is_dir():
                d_list, f_list = _safe_walk_tree(sr, root_real, violations, skip_top_dirs=frozenset())
                pkg_dirs.update(d_list)
                pkg_files.update(f_list)
            elif sr.is_file():
                pkg_files.add(sr)
        for pt in pkg.prod_target_files:
            if pt.is_file():
                pkg_files.add(pt)

        # Immediately reject ANY nested tests/ directory inside src/ — even when completely empty!
        for d in sorted(pkg_dirs):
            for sr in pkg.src_roots:
                if _is_within(d, sr) and d != sr:
                    rel_parts = d.relative_to(sr).parts
                    if "tests" in rel_parts:
                        rel_d = _to_posix_rel(d, root_real)
                        violations.append(
                            Violation(
                                "forbidden-tests-dir-in-src",
                                rel_d,
                                1,
                                "nested tests/ directory (even empty) is forbidden inside src/",
                            )
                        )
                        break

        for f in sorted(pkg_files):
            if f in scanned_src_files:
                continue
            scanned_src_files.add(f)
            rel_f = _to_posix_rel(f, root_real)

            if f.name in forbidden_file_names or f.name.endswith("_tests.rs") or f.name.endswith("_test.rs"):
                violations.append(
                    Violation(
                        "forbidden-test-filename-in-src",
                        rel_f,
                        1,
                        f"test/support filename '{f.name}' is forbidden under src/",
                    )
                )

            if _is_candidate_fixture_name(f):
                candidate_fixtures_in_src.add(rel_f)

            if f.suffix == ".rs":
                atts, bindings, markers, ptoks, ok = _scan_src_rs_file(
                    f,
                    root_real,
                    all_src_roots,
                    forbidden_suffixes,
                    pkg_forbidden_aliases,
                    violations,
                    crate_root=f in pkg.prod_target_files,
                )
                observed_attachments.extend(atts)
                for b in bindings:
                    mod_bindings_by_target.setdefault(b.resolved_target, []).append(b)
                include_markers.extend(markers)
                if ok:
                    prod_tokens_by_file[rel_f] = ptoks

    # 1. Verify reviewed_path_only_attachments exact bijection (reject duplicates too)
    reviewed_entries = policy["reviewed_path_only_attachments"]
    reviewed_map: dict[tuple[str, str, str, str], dict[str, Any]] = {}
    for idx, entry in enumerate(reviewed_entries):
        key = (entry["owner_file"], entry["module_path"], entry["mod_name"], entry["target"])
        if key in reviewed_map:
            violations.append(
                Violation(
                    "duplicate-reviewed-attachment",
                    "scripts/test_architecture_policy.json",
                    1,
                    f"duplicate reviewed_path_only_attachments entry at index {idx}: {key}",
                )
            )
        reviewed_map[key] = entry

    observed_by_id: dict[tuple[str, str, str, str], AttachmentRecord] = {}
    for att in observed_attachments:
        ident = att.identity()
        if ident in observed_by_id:
            violations.append(
                Violation(
                    "duplicate-observed-attachment",
                    att.owner_file,
                    att.line,
                    f"duplicate #[cfg(test)] #[path] attachment {ident}",
                )
            )
        observed_by_id[ident] = att
        if ident not in reviewed_map:
            violations.append(
                Violation(
                    "unreviewed-path-only-attachment",
                    att.owner_file,
                    att.line,
                    f"#[cfg(test)] #[path] mod {att.mod_name} -> {att.target} "
                    f"(module_path={att.module_path!r}) is missing from reviewed_path_only_attachments",
                )
            )

    for ident, entry in reviewed_map.items():
        if ident not in observed_by_id:
            violations.append(
                Violation(
                    "stale-reviewed-attachment",
                    entry["owner_file"],
                    1,
                    f"reviewed_path_only_attachments entry {ident} does not match any live source attachment",
                )
            )

    # Count actual reviewed attachments, not policy rows; count actual external
    # test roots containing Rust test source (support-only directories do not count).
    module_index = _ModuleIndex(root_real, packages, prod_tokens_by_file, mod_bindings_by_target)
    valid_attachments = []
    for a in observed_attachments:
        if not module_index.attachment_owner_reachable(a):
            violations.append(Violation("unreachable-reviewed-attachment", a.owner_file, a.line,
                                        "attachment owner is not in an actual Cargo target module graph"))
        elif a.identity() in reviewed_map:
            valid_attachments.append(a)
    test_roots: set[Path] = set()
    candidate_test_roots: set[Path] = set()
    test_modules: set[Path] = set()
    census_targets = {root_real / attachment.target for attachment in valid_attachments}
    integration_roots: set[Path] = set()
    for pkg in packages:
        if pkg.is_support:
            continue
        test_dir = pkg.dir_path / "tests"
        if test_dir.is_dir():
            _, test_files = _safe_walk_tree(test_dir, root_real, violations, skip_top_dirs=frozenset({"support"}))
            for f in test_files:
                if f.suffix != ".rs" or "support" in f.relative_to(test_dir).parts:
                    continue
                real = Path(os.path.realpath(f))
                if not _is_within(real, test_dir) or any(_is_within(real, sr) for sr in all_src_roots):
                    continue
                candidate_test_roots.add(test_dir)
                # Synthetic metadata-less fixtures use Cargo's conventional
                # integration root layout; the CLI uses actual metadata below.
                rel = f.relative_to(test_dir)
                if not cargo_metadata_used and (len(rel.parts) == 1 or
                        len(rel.parts) == 2 and rel.name == "main.rs"):
                    census_targets.add(real)
                    integration_roots.add(real)
        for target in pkg.metadata_targets:
            if "test" in target.get("kind", []):
                path = Path(os.path.realpath(target.get("src_path", "")))
                if _is_within(path, pkg.dir_path) and not any(_is_within(path, sr) for sr in all_src_roots):
                    census_targets.add(path)
                    integration_roots.add(path)
    # Census actual test functions reachable through reviewed attachments and
    # Cargo integration harnesses. This is not test enumeration/parity (BO-M9-C2).
    pending = sorted(census_targets, reverse=True)
    visited: set[Path] = set()
    while pending:
        f = pending.pop()
        if f in visited:
            continue
        visited.add(f)
        if _has_test_attribute(f, root_real, forbidden_suffixes, violations):
            test_modules.add(f)
        scratch: list[Violation] = []
        _, children, _, _, _ = _scan_src_rs_file(
            f, root_real, all_src_roots, set(), set(), scratch, external_inventory=True,
            crate_root=f in integration_roots,
        )
        # The same scanner's production-only findings are intentionally not
        # applied to external test sources; only validated child paths count.
        pending.extend(sorted({b.resolved_target for b in children} - visited, reverse=True))
    test_roots = {d for d in candidate_test_roots
                  if any(_is_within(f, d) and "support" not in f.relative_to(d).parts
                         for f in test_modules)}
    if not valid_attachments or not test_modules:
        violations.append(Violation("missing-test-modules", "Cargo.toml", 1, "requires a valid reviewed external attachment and actual test attribute in external test source"))
    if not test_roots:
        violations.append(Violation("missing-test-roots", "Cargo.toml", 1, "no non-support tests root containing a reachable test module"))

    # 2. Verify generated_sources provenance & include!/OUT_DIR bindings.
    gen_entries_by_path, gen_paths_in_src = _verify_generated(
        policy["generated_sources"], root_real, packages, all_src_roots,
        prod_tokens_by_file, module_index, violations,
    )
    for marker in include_markers:
        if marker.kind == "include_literal" and marker.resolved_target is not None:
            if not _is_within(marker.resolved_target, root_real):
                violations.append(Violation("include-escapes-workspace", marker.origin_rel, marker.line, "include target escapes workspace"))
                continue
            key = _to_posix_rel(marker.resolved_target, root_real)
            if "tests" in Path(key).parts or key.endswith(("_tests.rs", "_test.rs")):
                violations.append(Violation("include-test-source-in-src", marker.origin_rel, marker.line, f"include pulls test source {key}"))
                continue
        elif marker.kind == "include_out_dir":
            pkg = _package_for(root_real / marker.origin_rel, packages)
            key = _out_key(pkg, marker.raw_value) if pkg else None
        else:
            key = None
        entry = gen_entries_by_path.get(key)
        if entry is None or marker.origin_rel not in entry["consumers"]:
            code = "unapproved-include-target" if marker.kind == "include_literal" else "unclassified-generated-source"
            violations.append(Violation(code, marker.origin_rel, marker.line, f"no verified generated target/consumer for {marker.kind}: {key}"))

    # 3. Verify exceptional_production_fixtures & candidate coverage
    fixture_declared: dict[str, dict[str, Any]] = {}
    for entry in policy["exceptional_production_fixtures"]:
        f_rel = entry["path"]
        if f_rel in fixture_declared:
            violations.append(
                Violation("duplicate-production-fixture-entry", f_rel, 1, f"duplicate entry for '{f_rel}'")
            )
            continue
        fixture_declared[f_rel] = entry

        f_abs = Path(os.path.realpath(root_real / f_rel))
        if not _is_within(f_abs, root_real) or not f_abs.is_file():
            violations.append(
                Violation("stale-production-fixture-entry", f_rel, 1, f"entry '{f_rel}' does not exist")
            )
            continue
        if not any(_is_within(f_abs, Path(os.path.realpath(sr))) for sr in all_src_roots):
            violations.append(
                Violation("fixture-outside-src", f_rel, 1, f"entry '{f_rel}' is not under any src/ root")
            )
            continue

        owner_rel = entry["owner"]
        owner_abs = Path(os.path.realpath(root_real / owner_rel))
        if not _is_within(owner_abs, root_real) or not owner_abs.is_file():
            violations.append(
                Violation("missing-fixture-owner", f_rel, 1, f"owner '{owner_rel}' does not exist")
            )
            continue

        # Determine authoritative local module names bound to f_abs from its owner
        owner_bindings: list[ModBinding] = []
        if f_abs.suffix == ".rs":
            for b in mod_bindings_by_target.get(f_abs, []):
                if b.owner_rel == owner_rel:
                    owner_bindings.append(b)
            if not owner_bindings:
                violations.append(
                    Violation(
                        "unverified-fixture-owner",
                        f_rel,
                        1,
                        f"owner '{owner_rel}' does not declare a production module binding to '{f_rel}'",
                    )
                )
        else:
            owner_toks = prod_tokens_by_file.get(owner_rel, [])
            if not module_index.reachable(owner_rel) or not _tokens_use_asset_macro(owner_toks, owner_abs, f_abs, root_real, _package_for(owner_abs, packages)):
                violations.append(
                    Violation(
                        "unverified-fixture-owner",
                        f_rel,
                        1,
                        f"owner '{owner_rel}' does not load non-Rust asset '{f_rel}' via a recognized asset macro",
                    )
                )

        verified_prod_consumers = 0
        for cons in entry["consumers"]:
            c_abs = Path(os.path.realpath(root_real / cons))
            if not _is_within(c_abs, root_real) or not c_abs.is_file():
                violations.append(
                    Violation("missing-fixture-consumer", f_rel, 1, f"consumer '{cons}' does not exist")
                )
                continue
            in_prod_src = any(_is_within(c_abs, Path(os.path.realpath(sr))) for sr in all_src_roots)
            if not in_prod_src:
                violations.append(
                    Violation(
                        "non-production-fixture-consumer",
                        f_rel,
                        1,
                        f"consumer '{cons}' is outside src/",
                    )
                )
                continue
            cons_toks = prod_tokens_by_file.get(cons, [])
            if f_abs.suffix != ".rs":
                ok_ref = module_index.reachable(cons) and _tokens_use_asset_macro(cons_toks, c_abs, f_abs, root_real, _package_for(c_abs, packages))
            else:
                ok_ref = bool(owner_bindings) and module_index.consumes(cons, f_abs, owner_bindings)
            if ok_ref:
                verified_prod_consumers += 1
            else:
                violations.append(
                    Violation(
                        "unreferenced-fixture-consumer",
                        f_rel,
                        1,
                        f"listed production consumer '{cons}' has no verified production usage of '{f_rel}'",
                    )
                )

        if verified_prod_consumers == 0:
            violations.append(
                Violation(
                    "test-only-fixture-in-src",
                    f_rel,
                    1,
                    f"exceptional_production_fixtures entry '{f_rel}' has zero verified production consumers in src/",
                )
            )

    for cand_rel in sorted(candidate_fixtures_in_src):
        if cand_rel not in fixture_declared and cand_rel not in gen_paths_in_src:
            violations.append(
                Violation(
                    "unclassified-production-fixture",
                    cand_rel,
                    1,
                    f"candidate fixture/helper/non-Rust file '{cand_rel}' under src/ must be classified "
                    f"in exceptional_production_fixtures (or removed if test-only)",
                )
            )

    violations.sort(key=lambda v: (v.path, v.line, v.code))
    return {
        "status": "pass" if not violations else "fail",
        "root": str(root_real),
        "cargo_metadata_used": cargo_metadata_used,
        "workspace_members": [{"name": p.name, "rel_dir": p.rel_dir, "is_support": p.is_support} for p in packages],
        "src_roots": all_src_roots_rel,
        "test_support_crates": sorted(p.name for p in packages if p.is_support),
        "reviewed_path_only_attachments_count": len(observed_attachments),
        "test_module_count": len(test_modules),
        "tests_root_count": len(test_roots),
        "generated_sources_count": len(policy["generated_sources"]),
        "exceptional_production_fixtures_count": len(policy["exceptional_production_fixtures"]),
        "violation_count": len(violations),
        "violations": [v.to_dict() for v in violations],
    }


def _build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Canonical Rust workspace test-architecture guard")
    parser.add_argument("--root", type=Path, default=Path("."), help="Workspace root directory")
    parser.add_argument("--policy", type=Path, default=None, help="Override policy JSON path")
    parser.add_argument("--json", action="store_true", help="Print full JSON report")
    parser.add_argument(
        "--inventory",
        action="store_true",
        help="Include compact attachment inventory summary in text mode",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _build_arg_parser().parse_args(argv)
    report = evaluate_workspace(
        args.root,
        args.policy,
        skip_cargo_metadata=False,
    )
    if not report["cargo_metadata_used"]:
        # Defense in depth: production CLI never passes without authoritative cargo metadata
        if not any(v["code"] == "cargo-metadata-error" for v in report["violations"]):
            report["violations"].append(
                {
                    "code": "cargo-metadata-required",
                    "path": "Cargo.toml",
                    "line": 1,
                    "message": "authoritative cargo metadata is required for CLI evaluation",
                }
            )
            report["violation_count"] = len(report["violations"])
        report["status"] = "fail"

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(
            f"test-architecture: status={report['status']} "
            f"members={len(report['workspace_members'])} "
            f"src_roots={len(report['src_roots'])} "
            f"attachments={report['reviewed_path_only_attachments_count']} "
            f"test_modules={report['test_module_count']} tests_roots={report['tests_root_count']} "
            f"violations={report['violation_count']}"
        )
        if args.inventory:
            print(f"support_crates={report['test_support_crates']}")
            print(
                f"generated_sources={report['generated_sources_count']} "
                f"exceptional_fixtures={report['exceptional_production_fixtures_count']}"
            )
        for v in report["violations"]:
            print(f"  [{v['code']}] {v['path']}:{v['line']}: {v['message']}", file=sys.stderr)
    return 0 if report["violation_count"] == 0 and report["cargo_metadata_used"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
