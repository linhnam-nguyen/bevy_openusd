#!/usr/bin/env python3
"""Focused unit tests for H1 and M10 benchmark wrapper contracts."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import capture_h1_baseline  # noqa: E402
import capture_h1_c7  # noqa: E402
import m10_c2_compare  # noqa: E402
import m10_load_edit_matrix  # noqa: E402
import m10_memory_soak  # noqa: E402


def _make_h1_report(
    scenario: str,
    *,
    fixture: str = capture_h1_baseline.FIXTURE,
    config_ok: bool = True,
    steady_ok: bool = True,
    median_ms: float = 8.0,
    p95_ms: float = 10.0,
    fps: float = 120.0,
    semantic: dict | None = None,
) -> dict:
    return {
        "identity": {"scene_label": fixture, "scenario_code": scenario},
        "configuration_matches": config_ok,
        "steady_state_matches": steady_ok,
        "timing": {
            "median_frame_ms": median_ms,
            "p95_frame_ms": p95_ms,
            "actual_renderer_fps": fps,
        },
        "incident_semantic": semantic if semantic is not None else {},
        "incident_grid": {"prims_scanned": 10},
    }


def _full_h1_semantic() -> dict:
    return {field: 1.0 for field in sorted(capture_h1_c7.REQUIRED_SEMANTIC_FIELDS)}


def _make_m10_matrix(
    *,
    scene_hash: str = "abc123",
    backend: str = "vulkan",
    gpu: str = "RTX",
    cases: int = 16,
    cadences: int = 3,
    accepted: bool = True,
    cadence_match: bool = True,
) -> dict:
    return {
        "passed": True,
        "warmup_frames": 5,
        "measured_frames": 30,
        "identity": {
            "git_sha": "deadbeef",
            "scene_label": "Kitchen_set.usdz",
            "scene_hash": scene_hash,
            "width": 1920,
            "height": 1080,
            "backend": backend,
            "gpu_adapter": gpu,
        },
        "cases": [
            {"accepted": accepted, "configuration_matches": True} for _ in range(cases)
        ],
        "cadence_samples": [
            {
                "requested_fps": 30 * (idx + 1),
                "summary": {
                    "effective_renderer_target_fps": (
                        30 * (idx + 1) if cadence_match else 15
                    )
                },
            }
            for idx in range(cadences)
        ],
    }


def _make_m10_s1(
    *,
    scene_hash: str = "abc123",
    backend: str = "vulkan",
    gpu: str = "RTX",
    median_ms: float = 10.0,
    p95_ms: float = 12.0,
    fps: float = 60.0,
) -> dict:
    return {
        "identity": {
            "scene_label": "assets/external/Kitchen_set.usdz",
            "scene_hash": scene_hash,
            "width": 1920,
            "height": 1080,
            "build_profile": "release",
            "backend": backend,
            "gpu_adapter": gpu,
        },
        "configuration_matches": True,
        "steady_state_matches": True,
        "timing": {
            "median_frame_ms": median_ms,
            "p95_frame_ms": p95_ms,
            "actual_renderer_fps": fps,
            "gpu_median_frame_ms": 4.0,
            "gpu_p95_frame_ms": 5.0,
        },
    }


class TestCaptureH1Baseline(unittest.TestCase):
    def test_validate_report_accepts_valid_s1_and_s11(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "native-s1.json"
            path.write_text(json.dumps(_make_h1_report("S1")), encoding="utf-8")
            parsed = capture_h1_baseline.validate_report(path, "S1")
            self.assertEqual(parsed["scenario"], "S1")
            self.assertEqual(parsed["median_frame_ms"], 8.0)

    def test_validate_report_rejects_identity_and_invariant_errors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "bad.json"
            cases = [
                _make_h1_report("S1", fixture="assets/external/other.usdz"),
                _make_h1_report("S11"),
                _make_h1_report("S1", config_ok=False),
                _make_h1_report("S1", steady_ok=False),
                _make_h1_report("S1", median_ms="nan"),  # type: ignore[arg-type]
            ]
            for payload in cases:
                path.write_text(json.dumps(payload), encoding="utf-8")
                with self.assertRaises(ValueError):
                    capture_h1_baseline.validate_report(path, "S1")

    def test_main_writes_c2_baseline_packet(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = Path(tmp) / "out"

            def fake_capture(scenario: str, output: Path, warmup: int, frames: int) -> None:
                output.write_text(
                    json.dumps(_make_h1_report(scenario)), encoding="utf-8"
                )

            with (
                mock.patch.object(capture_h1_baseline, "capture", side_effect=fake_capture),
                mock.patch.object(capture_h1_baseline, "git_sha", return_value="abc1234"),
                mock.patch.object(
                    sys,
                    "argv",
                    ["capture_h1_baseline.py", "--output-dir", str(out_dir), "--warmup", "5", "--frames", "10"],
                ),
            ):
                rc = capture_h1_baseline.main()

            self.assertEqual(rc, 0)
            packet = json.loads((out_dir / "baseline.json").read_text(encoding="utf-8"))
            self.assertEqual(packet["schema"], "usdhub.h1.c2.postupdate-baseline.v1")
            self.assertEqual([case["scenario"] for case in packet["cases"]], ["S1", "S11"])
            self.assertEqual(packet["client_evidence"]["status"], "not_applicable_to_C2")


class TestCaptureH1C7(unittest.TestCase):
    def test_main_writes_regression_packet_and_checks_telemetry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            fake_root = Path(tmp)
            c2_dir = fake_root / "target" / "benchmark" / "h1-c2-baseline"
            c2_dir.mkdir(parents=True)
            c2_dir.joinpath("baseline.json").write_text(
                json.dumps(
                    {
                        "cases": [
                            {"scenario": "S1", "median_frame_ms": 10.0, "p95_frame_ms": 20.0},
                            {"scenario": "S11", "median_frame_ms": 12.0, "p95_frame_ms": 25.0},
                        ]
                    }
                ),
                encoding="utf-8",
            )
            out_dir = fake_root / "out"

            def fake_capture(scenario: str, output: Path, warmup: int, frames: int) -> None:
                output.write_text(
                    json.dumps(
                        _make_h1_report(
                            scenario,
                            median_ms=9.0 if scenario == "S1" else 12.0,
                            p95_ms=18.0 if scenario == "S1" else 25.0,
                            semantic=_full_h1_semantic(),
                        )
                    ),
                    encoding="utf-8",
                )

            with (
                mock.patch.object(capture_h1_c7, "ROOT", fake_root),
                mock.patch.object(capture_h1_c7, "capture", side_effect=fake_capture),
                mock.patch.object(capture_h1_c7, "git_sha", return_value="def5678"),
                mock.patch.object(
                    sys,
                    "argv",
                    ["capture_h1_c7.py", "--output-dir", str(out_dir)],
                ),
            ):
                rc = capture_h1_c7.main()

            self.assertEqual(rc, 0)
            packet = json.loads((out_dir / "regression.json").read_text(encoding="utf-8"))
            self.assertEqual(packet["schema"], "usdhub.h1.c7.runtime-isolation-regression.v1")
            s1_case = packet["cases"][0]
            self.assertAlmostEqual(s1_case["median_frame_delta_percent"], -10.0)
            self.assertAlmostEqual(s1_case["p95_frame_delta_percent"], -10.0)

    def test_main_rejects_missing_semantic_telemetry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            fake_root = Path(tmp)
            out_dir = fake_root / "out"

            def fake_capture(scenario: str, output: Path, warmup: int, frames: int) -> None:
                output.write_text(
                    json.dumps(_make_h1_report(scenario, semantic={})), encoding="utf-8"
                )

            with (
                mock.patch.object(capture_h1_c7, "ROOT", fake_root),
                mock.patch.object(capture_h1_c7, "capture", side_effect=fake_capture),
                mock.patch.object(sys, "argv", ["capture_h1_c7.py", "--output-dir", str(out_dir)]),
                self.assertRaises(ValueError),
            ):
                capture_h1_c7.main()


class TestM10C2Compare(unittest.TestCase):
    def test_validate_pair_rejects_identity_and_matrix_mismatches(self) -> None:
        base_m = _make_m10_matrix()
        cand_m = _make_m10_matrix()
        base_s1 = _make_m10_s1()
        cand_s1 = _make_m10_s1()
        m10_c2_compare.validate_pair(base_m, cand_m, base_s1, cand_s1)

        with self.assertRaises(ValueError):
            m10_c2_compare.validate_pair(base_m, _make_m10_matrix(scene_hash="diff"), base_s1, cand_s1)
        with self.assertRaises(ValueError):
            m10_c2_compare.validate_pair(base_m, _make_m10_matrix(cases=15), base_s1, cand_s1)
        with self.assertRaises(ValueError):
            m10_c2_compare.validate_pair(base_m, _make_m10_matrix(cadence_match=False), base_s1, cand_s1)
        with self.assertRaises(ValueError):
            m10_c2_compare.validate_pair(base_m, cand_m, base_s1, _make_m10_s1(gpu="OtherGPU"))

    def test_main_writes_report_and_enforces_regression_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bm = root / "bm.json"
            cm = root / "cm.json"
            bs = root / "bs.json"
            cs = root / "cs.json"
            out = root / "compare.json"
            bm.write_text(json.dumps(_make_m10_matrix()), encoding="utf-8")
            cm.write_text(json.dumps(_make_m10_matrix()), encoding="utf-8")
            bs.write_text(json.dumps(_make_m10_s1(median_ms=10.0, p95_ms=12.0, fps=60.0)), encoding="utf-8")
            cs.write_text(json.dumps(_make_m10_s1(median_ms=11.0, p95_ms=12.0, fps=58.0)), encoding="utf-8")

            argv = [
                "m10_c2_compare.py",
                "--baseline-matrix", str(bm),
                "--candidate-matrix", str(cm),
                "--baseline-s1", str(bs),
                "--candidate-s1", str(cs),
                "--output", str(out),
                "--max-regression-percent", "12.0",
            ]
            with mock.patch.object(sys, "argv", argv):
                self.assertEqual(m10_c2_compare.main(), 0)
            report = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(report["schema"], "usdhub.m10.c2.representative-comparison.v1")
            self.assertTrue(report["regression_gate"]["passed"])

            argv[-1] = "5.0"
            with mock.patch.object(sys, "argv", argv), self.assertRaises(ValueError):
                m10_c2_compare.main()


class TestM10LoadEditMatrix(unittest.TestCase):
    def test_main_builds_11_row_matrix_from_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            art = root / "artifacts"
            art.mkdir(parents=True)
            for label in m10_load_edit_matrix.INITIAL_FIXTURES:
                (art / f"m10-c3-initial-{label}.json").write_text(
                    json.dumps(
                        {
                            "configuration_matches": True,
                            "steady_state_matches": True,
                            "identity": {"git_sha": "abc"},
                            "phase_metrics": {"initial_projection_ms": 5.0, "initial_projection_prims": 42},
                            "cache_snapshot": {"live_stage_prims": 42, "cached_materials": 1, "cached_textures": 0},
                            "timing": {"median_frame_ms": 8.0, "p95_frame_ms": 9.0, "actual_renderer_fps": 60.0},
                        }
                    ),
                    encoding="utf-8",
                )
            (art / "m10-c3-full-fallback.json").write_text(
                json.dumps(
                    {
                        "identity": {"scene_label": "fallback.usda", "git_sha": "abc"},
                        "incident_semantic": {"fallback_extractions": 2, "snapshot_clones": 1},
                        "incident_grid": {"prims_scanned": 42, "compute_extent_calls": 1},
                        "cache_snapshot": {"live_stage_prims": 42, "cached_materials": 1, "cached_textures": 0},
                        "phase_metrics": {"initial_projection_ms": 6.0, "initial_projection_prims": 42},
                        "raw_samples": [1, 2],
                        "timing": {"median_frame_ms": 8.0, "p95_frame_ms": 9.0, "actual_renderer_fps": 60.0},
                    }
                ),
                encoding="utf-8",
            )
            target_dir = root / "target"
            target_dir.mkdir()
            (target_dir / "m8-c1-instancing-baseline.json").write_text(
                json.dumps(
                    {
                        "fixture": "tests/fixtures/usd/point_instancer.usda",
                        "projection_ms": 12.0,
                        "point_instancer_prim_count": 100,
                        "mesh_entity_count": 100,
                        "mesh_asset_count": 1,
                        "material_asset_count": 1,
                        "logical_instance_count": 100,
                        "unique_mesh_handles": 1,
                        "git_sha": "abc",
                    }
                ),
                encoding="utf-8",
            )
            op_names = [
                "xformOp:translate",
                "visibility",
                "material:binding",
                "points",
                "primvars:displayColor",
                "subtree-add",
                "subtree-remove",
            ]
            (target_dir / "m5-c4-live-mesh-patch.json").write_text(
                json.dumps(
                    {
                        "operations": [
                            {
                                "operation": name,
                                "patch_latency_ms": 1.5,
                                "mesh_conversions": 0,
                                "source_cache_lookups": 1,
                                "source_cache_hits": 1,
                                "source_cache_misses": 0,
                                "reconcile_visited_stage_prims": 2,
                                "reconcile_patched_entities": 1,
                                "reconcile_spawned_entities": 0,
                                "reconcile_despawned_entities": 0,
                            }
                            for name in op_names
                        ]
                    }
                ),
                encoding="utf-8",
            )
            out = root / "matrix.json"
            with (
                mock.patch.object(m10_load_edit_matrix, "ROOT", root),
                mock.patch.object(
                    m10_load_edit_matrix.subprocess, "check_output", return_value="abc123\n"
                ),
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        "m10_load_edit_matrix.py",
                        "--skip-loads",
                        "--artifact-dir",
                        str(art),
                        "--output",
                        str(out),
                    ],
                ),
            ):
                rc = m10_load_edit_matrix.main()

            self.assertEqual(rc, 0)
            matrix = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(matrix["schema"], "usdhub.m10.c3.load-edit-matrix.v1")
            self.assertEqual(len(matrix["initial_load_rows"]), 5)
            self.assertEqual(len(matrix["live_edit_rows"]), 6)

    def test_full_fallback_row_requires_nonzero_fallback(self) -> None:
        with self.assertRaises(ValueError):
            m10_load_edit_matrix.full_fallback_row({"incident_semantic": {"fallback_extractions": 0}})


class TestM10MemorySoak(unittest.TestCase):
    def test_rejects_cycles_below_twelve(self) -> None:
        with mock.patch.object(sys, "argv", ["m10_memory_soak.py", "--cycles", "11"]):
            with self.assertRaises(SystemExit):
                m10_memory_soak.main()

    def test_main_writes_v2_soak_report_and_requires_persistent_app(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            runtime_artifact = root / "m10-c4-persistent-runtime.json"
            runtime_artifact.write_text(
                json.dumps({"passed": True, "persistent_app": True, "cycles": 12}),
                encoding="utf-8",
            )
            out = root / "soak.json"
            fake_proc = mock.MagicMock()
            fake_proc.pid = 4242
            fake_proc.returncode = 0
            fake_proc.poll.return_value = 0
            fake_proc.stdout.read.return_value = "ok"

            with (
                mock.patch.object(m10_memory_soak, "RUNTIME_ARTIFACT", runtime_artifact),
                mock.patch.object(m10_memory_soak.subprocess, "Popen", return_value=fake_proc),
                mock.patch.object(m10_memory_soak, "process_tree_rss_kb", return_value=2048),
                mock.patch.object(sys, "argv", ["m10_memory_soak.py", "--cycles", "12", "--output", str(out)]),
            ):
                rc = m10_memory_soak.main()

            self.assertEqual(rc, 0)
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(payload["schema"], "usdhub.m10.c4.memory-soak.v2")
            self.assertEqual(payload["rss_high_water_kb"], 2048)

            runtime_artifact.write_text(
                json.dumps({"passed": True, "persistent_app": False}), encoding="utf-8"
            )
            with (
                mock.patch.object(m10_memory_soak, "RUNTIME_ARTIFACT", runtime_artifact),
                mock.patch.object(m10_memory_soak.subprocess, "Popen", return_value=fake_proc),
                mock.patch.object(m10_memory_soak, "process_tree_rss_kb", return_value=2048),
                mock.patch.object(sys, "argv", ["m10_memory_soak.py", "--cycles", "12", "--output", str(out)]),
                self.assertRaises(SystemExit),
            ):
                m10_memory_soak.main()


if __name__ == "__main__":
    unittest.main()
