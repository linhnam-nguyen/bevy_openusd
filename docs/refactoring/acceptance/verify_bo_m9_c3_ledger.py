"""Read-only, offline verifier for the BO-M9-C3 correction evidence.

Run at the exact backend C3 HEAD; this is a documentation audit, not a
replacement for Cargo parity, formal Test, hosted CI or native smoke.
"""
from __future__ import annotations

import argparse
import collections
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys

C0 = "1cbfd736279ae761b3ad4ae31e5708a84f43c0a5"
C1 = "59ccddbea7be85a34a8e8f1d4d131c91c7ade858"
C3 = "7097ae866a522a96cc67cfa5d933930e368002cb"
M7 = "a45daf853c9330832e69e71d9a370958467548fc"
M8 = "2ac26f305f9a5201ed005b1d136f876c583e7975"
CENSUS = "docs/refactoring/backend-test-census.md"
FLAT = "scripts/test_enumeration_baseline.json"
QUALIFIED = "scripts/test_enumeration_qualified_baseline.json"
ACCEPTANCE = "docs/refactoring/acceptance"
CONFIGS = {f"{os}:{feature}" for os in ("windows", "linux", "macos")
           for feature in ("default", "solari")}
BUCKETS = ("common_tests", "windows_only_tests", "unix_only_tests", "solari_feature_tests")
ADDITIONS = {
    "canonical_test_architecture_guard_self_tests_pass": "Cargo.toml::test::test_architecture",
    "workspace_satisfies_canonical_test_architecture_guard": "Cargo.toml::test::test_architecture",
    "test_configured_initial_branch": "tests/support/project/Cargo.toml::lib::project_support",
    "test_create_layered_test_repository": "tests/support/project/Cargo.toml::lib::project_support",
    "test_create_test_repository": "tests/support/project/Cargo.toml::lib::project_support",
    "test_deep_overlap_stage_structure": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_model_usda_fixture_validity": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_resolve_asset_and_external_paths": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_resolve_crate_fixture_path": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_resolve_fixture_path": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_synthetic_wide_stage_structure": "tests/support/usd/Cargo.toml::lib::usd_support",
    "test_semantic_test_app_initialization": "tests/support/viewport/Cargo.toml::lib::viewport_support",
    "test_sorted_paths": "tests/support/viewport/Cargo.toml::lib::viewport_support",
}
# Exact function-name transforms from BO-M8 commit, not fuzzy matching.
RENAMES = {
    "m5_c4_live_patch_matrix_keeps_unrelated_edits_out_of_mesh_conversion": "live_patch_matrix_keeps_unrelated_edits_out_of_mesh_conversion",
    "records_m6_shared_material_benchmark_artifact": "records_shared_material_benchmark_artifact",
    "records_m10_c4_persistent_runtime_soak": "records_persistent_runtime_soak",
    "records_m8_c1_point_instancer_baseline": "records_point_instancer_baseline",
    "records_m8_c6_instancing_freeze": "records_point_instancer_reconcile_profile",
    "records_m7_progressive_load_benchmark_artifact": "records_progressive_load_benchmark_artifact",
    "profiles_m25_o10_empirical_benchmark_suite": "profiles_subtree_resync_empirical_suite",
    "phase2_freeze_matrix_covers_create_import_composition_and_recovery": "end_to_end_create_import_composition_and_recovery",
    "phase2_freeze_matrix_covers_native_and_adopted_imports_and_missing_locations": "end_to_end_native_and_adopted_imports_and_missing_locations",
    "c1_inventory_freezes_the_three_canonical_fixture_records": "inventory_freezes_the_three_canonical_fixture_records",
    "c1_frozen_fixture_paths_are_not_derived_from_display_names": "frozen_fixture_paths_are_not_derived_from_display_names",
    "c1_smoke_uses_only_the_first_four_frozen_seeds": "smoke_uses_only_the_first_four_frozen_seeds",
    "c7_smoke_scopes_cloned_project_registration_for_four_seeds": "smoke_scopes_cloned_project_registration_for_four_seeds",
    "c8_runs_the_full_projects_lifecycle_matrix_for_all_sixteen_seeds": "runs_the_full_projects_lifecycle_matrix_for_all_sixteen_seeds",
    "c2_freezes_proj_t_hierarchy_and_scoped_identity_lookup": "freezes_proj_t_hierarchy_and_scoped_identity_lookup",
    "c3_smoke_exercises_seeded_import_link_composition_for_four_seeds": "smoke_exercises_seeded_import_link_composition_for_four_seeds",
    "c4_smoke_verifies_three_scoped_scene_activations_for_four_seeds": "smoke_verifies_three_scoped_scene_activations_for_four_seeds",
    "c6_smoke_round_trips_exported_scenes_for_four_seeds": "smoke_round_trips_exported_scenes_for_four_seeds",
    "c5_smoke_proves_scene_label_reuse_and_identity_for_four_seeds": "smoke_proves_scene_label_reuse_and_identity_for_four_seeds",
    "m3_edit_semantic_diff_undo_redo_save_replace_and_batch_converge": "semantic_edit_diff_undo_redo_save_replace_and_batch_converge",
    "production_selection_follows_distinct_child_members_through_c7_repair": "production_selection_follows_distinct_child_members_through_repair",
    "test_m25_o11_milestone_acceptance_unaffected_sibling_pipeline_invariance": "unaffected_sibling_pipeline_preserves_resync_invariance",
}
MOVES = {
    ("crates/project_protocol/Cargo.toml::lib::project_protocol", "crates/project_protocol/Cargo.toml::test::contracts"): 21,
    ("crates/viewport_protocol/Cargo.toml::lib::viewport_protocol", "crates/viewport_protocol/Cargo.toml::test::relocated_contracts"): 12,
    ("crates/viewport_streaming/Cargo.toml::lib::viewport_streaming", "crates/viewport_streaming/Cargo.toml::test::relocated_contracts"): 18,
    ("Cargo.toml::test::m8_instancing_correctness", "Cargo.toml::test::point_instancer_correctness"): 7,
    ("Cargo.toml::test::m10_persistent_soak", "Cargo.toml::test::persistent_runtime_soak"): 1,
    ("Cargo.toml::test::m8_instancing_freeze", "Cargo.toml::test::point_instancer_reconcile_profile"): 1,
    ("Cargo.toml::test::m8_instancing_profile", "Cargo.toml::test::point_instancer_profile"): 1,
}


def require(ok, detail):
    if not ok:
        raise ValueError(detail)


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE)


def blob_sha(data):
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def read_tsv(path):
    with path.open(encoding="utf-8", newline="") as file:
        return list(csv.DictReader(file, delimiter="\t"))


def census_rows(text):
    text = text.split("## Cargo target static test identities", 1)[1].split("## Final Compile result", 1)[0]
    out = {}
    for line in text.splitlines():
        if not re.match(r"^\| (crates/|src/|tests/)", line):
            continue
        cells = [x.strip() for x in line.split("|")]
        path = cells[1]
        for name in cells[-2].split(";"):
            name = name.strip()
            note = re.search(r" \[([^]]+)\]$", name)
            name = re.sub(r" \[[^]]+\]$", "", name)
            require(re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name), (path, name))
            key = (path, name)
            require(key not in out, f"duplicate C0 identity {key}")
            out[key] = ("Dormant/unregistered" in line, note.group(1) if note else "")
    require(len(out) == 1236, f"C0 identity count {len(out)}")
    require(sum(d for d, _ in out.values()) == 45, "C0 dormant count")
    return out


def expected_old_targets(path):
    if path.startswith("src/"):
        return "Cargo.toml::bin::usdview;Cargo.toml::lib::usdview"
    if path.startswith("crates/"):
        package = path.split("/")[1]
        manifest = f"crates/{package}/Cargo.toml"
        if "/src/" in path:
            return f"{manifest}::lib::{package}"
        target = path.split("/tests/", 1)[1].split("/")[0].removesuffix(".rs")
        return f"{manifest}::test::{target}"
    require(path.startswith("tests/"), path)
    return "Cargo.toml::test::" + path.split("/")[1].removesuffix(".rs")


def validate_identity(rows, original, flat, qualified):
    require(flat["baseline_commit"] == qualified["baseline_commit"] == C1, "wrong C1 baseline")
    require(set(qualified["configurations"]) == CONFIGS, "configuration set mismatch")
    names = {}
    for bucket in BUCKETS:
        for item in flat[bucket]:
            require(item["name"] not in names, "duplicate flat identity")
            names[item["name"]] = (bucket, item["count"])
    require(len(names) == 1204, "flat identity union mismatch")
    actual = {}
    by_name = collections.defaultdict(dict)
    for config, cap in qualified["configurations"].items():
        require(cap["baseline_commit"] == C1 and cap["configuration"] == config, config)
        if config.startswith(("linux:", "macos:")):
            require(cap.get("native_host_verified") is False and cap.get("capture_provenance") == "DRAFT_SYNTHETIC_WINDOWS_DERIVED_AWAITING_NATIVE_HOST", "draft provenance lost")
        observed = collections.Counter()
        target_keys = set()
        for target in cap["targets"]:
            key = (target["manifest"], ",".join(target["kind"]), target["target"])
            require(key not in target_keys, (config, key, "duplicate target"))
            target_keys.add(key)
            for line in target["listing"].splitlines():
                if line.endswith(": test"):
                    observed[(*key, line[:-6])] += 1
                elif line:
                    require(re.fullmatch(r"\d+ tests?, 0 benchmarks?", line), (config, key, line))
        require(len(target_keys) == 43, "target count mismatch")
        require(all(v == 1 for v in observed.values()), "duplicate qualified identity")
        actual[config] = observed
        for (manifest, kind, target, name), count in observed.items():
            by_name[name].setdefault(config, set()).add("::".join((manifest, kind, target)))
    require(set(by_name) == set(names), "qualified/flat union mismatch")
    seen_original, seen_current, seen_added, seen_renames = set(), set(), set(), set()
    moves = collections.Counter()
    projected = {c: collections.Counter() for c in CONFIGS}
    for row in rows:
        origin = row["origin"]
        old = row["old_path"], row["old_function"]
        if origin.startswith("C0_"):
            require(old in original and old not in seen_original, f"invalid/duplicate C0 source {old}")
            seen_original.add(old)
            dormant, note = original[old]
            require(dormant == (origin == "C0_DORMANT"), (old, "dormant status mismatch"))
            if dormant:
                require(row["new_function"] == old[1] and row["bucket"] == "dormant" and not any(row[k] for k in ("new_name", "old_targets", "new_targets", "configurations")), (old, "dormant identity fabricated or changed"))
                continue
            require(origin == "C0_ACTIVE" and row["old_targets"] == expected_old_targets(old[0]), (old, "original target mismatch"))
            expected_leaf = RENAMES.get(old[1], old[1])
            if old[1] in RENAMES:
                seen_renames.add(old[1])
            if note == "Windows":
                require(row["bucket"] == "windows_only_tests", old)
            if note == "non-Windows":
                require(row["bucket"] == "unix_only_tests", old)
            if note == "all supported targets":
                require(row["bucket"] == "common_tests", old)
            if row["old_targets"] != row["new_targets"]:
                moves[(row["old_targets"], row["new_targets"])] += 1
            old_manifests = {k.split("::")[0] for k in row["old_targets"].split(";")}
            require(old_manifests == {k.split("::")[0] for k in row["new_targets"].split(";")}, (old, "owning package changed"))
        else:
            expected_leaf = row["new_function"]
            require(not any(old) and not row["old_targets"], "invented original for addition")
            require(expected_leaf in ADDITIONS and expected_leaf not in seen_added, f"unapproved addition {expected_leaf}")
            require(row["new_targets"] == ADDITIONS[expected_leaf], f"wrong addition target {expected_leaf}")
            require(origin == ("BO-M9-C1_ADDED" if "test_architecture" in row["new_targets"] else "BO-M6_M7_ADDED"), "wrong addition origin")
            seen_added.add(expected_leaf)
        name = row["new_name"]
        require(name in names and name not in seen_current, f"missing/duplicate current identity {name}")
        seen_current.add(name)
        require(row["new_function"] == name.rsplit("::", 1)[-1] == expected_leaf, (old, name, "leaf/rename mismatch"))
        bucket, multiplicity = names[name]
        require(row["bucket"] == bucket, (name, "bucket mismatch"))
        eligible = set(row["configurations"].split(";"))
        expected_configs = {c for c in CONFIGS if bucket == "common_tests" or
                            bucket == "windows_only_tests" and c.startswith("windows:") or
                            bucket == "unix_only_tests" and not c.startswith("windows:") or
                            bucket == "solari_feature_tests" and c.endswith(":solari")}
        require(eligible == expected_configs == set(by_name[name]), (name, "eligibility mismatch"))
        target_set = set(row["new_targets"].split(";"))
        for config in eligible:
            require(target_set == by_name[name][config] and len(target_set) == multiplicity, (name, config, "target mismatch"))
            for target in target_set:
                projected[config][(*target.split("::"), name)] += 1
    require(len(rows) == 1249 and seen_original == set(original) and seen_current == set(names), "identity coverage failure")
    require(seen_added == set(ADDITIONS) and seen_renames == set(RENAMES), "addition/rename coverage failure")
    require(moves == MOVES, f"unauthorized target move: {moves}")
    for config in CONFIGS:
        require(projected[config] == actual[config], f"qualified multiset mismatch {config}")
    return {c: sum(actual[c].values()) for c in sorted(CONFIGS)}


def verify_visibility(rows, raw):
    expected = {}
    for file in raw:
        for side in ("added", "removed"):
            for i, hit in enumerate(file[side], 1):
                expected[(file["file"], side, str(i))] = hit
    require(len(expected) == len(rows) == 158, "visibility record count")
    seen, counts = set(), collections.Counter()
    for row in rows:
        key = row["file"], row["side"], row["ordinal"]
        require(key in expected and key not in seen, f"unknown/duplicate raw hit {key}")
        seen.add(key)
        hit = expected[key]
        require(row["snippet"] == hit["snippet"] and row["visibility"] == hit["vis"] and row["kind"] == hit["kind"], (key, "raw hit changed"))
        require(row["line"] == ("" if hit.get("line") is None else str(hit["line"])) and row["cfg_test"] == ("" if hit.get("cfg_test") is None else str(hit["cfg_test"]).lower()), (key, "line/cfg changed"))
        require(bool(row["rationale"]), (key, "missing disposition"))
        if row["side"] == "removed":
            require(row["classification"] in {"PUBLIC_IDENTICAL_REEXPORT", "RESTRICTED_REORDER_REFLOW_OR_SAME_VISIBILITY", "DOCUMENTED_PRIVATE_REPLACEMENT", "OTHER_REMOVED_RESTRICTED_LINE"}, (key, "unsupported removal classification"))
        if hit["vis"] == "pub":
            require(row["classification"] == "PUBLIC_IDENTICAL_REEXPORT" and hit["snippet"] == "pub use transform::extract_transform;" and row["file"] == "crates/usd_semantic/src/lib.rs", "public change")
        if hit.get("cfg_test") is True:
            require(row["classification"] == "TEST_GATED_PATH_ATTACHMENT", (key, "cfg-test disposition"))
        counts[(row["side"], row["classification"])] += 1
    require(seen == set(expected), "unaccounted raw record")
    require(counts == {("added", "PUBLIC_IDENTICAL_REEXPORT"): 1,
                       ("added", "RESTRICTED_REORDER_REFLOW_OR_SAME_VISIBILITY"): 15,
                       ("added", "WINDOWS_MIGRATION_GUARD"): 4,
                       ("added", "PRIVATE_COLLABORATOR_OR_REPLACEMENT"): 22,
                       ("added", "TEST_GATED_PATH_ATTACHMENT"): 18,
                       ("removed", "PUBLIC_IDENTICAL_REEXPORT"): 1,
                       ("removed", "RESTRICTED_REORDER_REFLOW_OR_SAME_VISIBILITY"): 15,
                       ("removed", "DOCUMENTED_PRIVATE_REPLACEMENT"): 6,
                       ("removed", "OTHER_REMOVED_RESTRICTED_LINE"): 76}, "visibility classification totals")
    return {"added_raw_hits": 60, "removed_raw_hits": 98, "unconditional_public_hit": 1,
            "restricted_hits": 41, "test_gated_hits": 18, "semantic_test_only_removals": "NOT_COUNTED"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args()
    root = args.root.resolve()
    require(git(root, "rev-parse", "HEAD").decode().strip() == C3, "exact C3 HEAD required")
    require(not git(root, "status", "--porcelain", "--untracked-files=all", "--", "src", "crates", "tests", "scripts", "Cargo.toml", "Cargo.lock", ".github").strip(), "product source/manifest/CI changed; review before proceeding")
    census = git(root, "show", f"{C0}:{CENSUS}")
    require(blob_sha(census) == "8e8b7d14acc2d7cdcf1cad6c6ff10c1ec66cb079", "C0 census blob mismatch")
    flat = git(root, "show", f"{C3}:{FLAT}")
    qualified = git(root, "show", f"{C3}:{QUALIFIED}")
    require(blob_sha(flat) == "f0e323a1744a8e3f2cb758f68122a07ece4219be", "flat baseline blob mismatch")
    require(blob_sha(qualified) == "3bbf4306ddb1dbe8ed988d6bab2d0595776ad913", "qualified baseline blob mismatch")
    original = census_rows(census.decode("utf-8"))
    source = root / ACCEPTANCE
    rows = read_tsv(source / "bo-m9-c3-test-identity-ledger.tsv")
    totals = validate_identity(rows, original, json.loads(flat), json.loads(qualified))
    dormant_by_path = collections.defaultdict(list)
    for (path, name), (dormant, _) in original.items():
        if dormant:
            dormant_by_path[path].append(name)
    require(len(dormant_by_path) == 37 and sum(len(v) for v in dormant_by_path.values()) == 45, "dormant path/definition count")
    unchanged_dormant = 0
    m7_normalized_dormant = 0
    for path, d_names in sorted(dormant_by_path.items()):
        c3_src = git(root, "show", f"{C3}:{path}").decode("utf-8")
        for name in d_names:
            require(
                re.search(r"#\[(?:[\w:]*test)\b[^\]]*\](?:\s*#\[[^\]]*\])*\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+" + re.escape(name) + r"\s*\(", c3_src),
                f"dormant test definition missing at C3: {path}::{name}",
            )
        if git(root, "rev-parse", f"{C0}:{path}") == git(root, "rev-parse", f"{C3}:{path}"):
            unchanged_dormant += 1
        else:
            commits = git(root, "log", "--format=%H", f"{C0}..{C3}", "--", path).decode("utf-8").split()
            require(commits == [M7], f"dormant source changed outside BO-M7 ({commits}): {path}")
            m7_normalized_dormant += 1
    m8_diff = git(root, "show", "--format=", "--find-renames", "--no-ext-diff", "--unified=0", M8, "--", "src", "crates", "tests").decode("utf-8")
    for old, new in RENAMES.items():
        require(re.search(r"^-\s*(?:async\s+)?fn\s+" + re.escape(old) + r"\s*\(", m8_diff, re.M) and
                re.search(r"^\+\s*(?:async\s+)?fn\s+" + re.escape(new) + r"\s*\(", m8_diff, re.M), f"BO-M8 rename evidence missing: {old}")
    for ref in (C0, C3):
        probe = git(root, "show", f"{ref}:src/project/service/stage_activation.rs").decode("utf-8")
        require("/// ```compile_fail" in probe and "requires_send::<openusd::usd::Stage>();" in probe and "pub(crate) const OPENUSD_STAGE_SEND_PROBE" in probe, f"compile-fail probe missing at {ref}")
    raw = (source / "bo-m9-c3-visibility-classified.json").read_bytes()
    require(hashlib.sha256(raw.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")).hexdigest() == "dc5e213893ec8c56f2bf3dd30029340d2842708e4e56c8f294128dc679b9047c", "visibility source hash mismatch")
    audit = verify_visibility(read_tsv(source / "bo-m9-c3-visibility-hit-ledger.tsv"), json.loads(raw))
    print(json.dumps({"status": "pass", "C0": C0, "C1": C1, "C3": C3,
                      "original_definitions": 1236, "mapped_registered_definitions": 1191,
                      "dormant_definitions": 45, "dormant_paths": 37,
                      "unchanged_dormant_blobs": unchanged_dormant,
                      "m7_fixture_normalized_dormant_blobs": m7_normalized_dormant,
                      "approved_additions": 13, "current_definitions": 1249,
                      "function_renames": 22, "target_moves_or_renames": 61,
                      "qualified_occurrences": totals, "visibility": audit,
                      "scope": "static ledger and exact recorded baselines; not live Cargo, native smoke, hosted CI or formal Test"}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, IndexError, OSError, subprocess.CalledProcessError) as exc:
        print(f"C3 evidence verification FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
