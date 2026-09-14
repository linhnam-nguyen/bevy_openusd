import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "animation_debug.py"
SPEC = importlib.util.spec_from_file_location("animation_debug", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def scenario(scenario_id, status=MODULE.PASS, failure_code=None):
    return {"id": scenario_id, "status": status, "failure_code": failure_code}


def matrix_rows():
    return [
        MODULE.matrix_row(entry["id"], entry["description"], scenario(entry["scenario"]))
        for entry in MODULE.MATRIX
    ]


class AnimationDebugMatrixTests(unittest.TestCase):
    def test_all_required_rows_and_gates_are_green_only_together(self):
        fault = MODULE.fault_verification("test_fault", "CLIENT_DECODE_STALLED", "CLIENT_DECODE_STALLED")
        self.assertEqual(
            MODULE.overall_status(matrix_rows(), [], [], fault), MODULE.PASS
        )

    def test_unavailable_client_row_is_not_a_pass(self):
        rows = matrix_rows()
        rows[2] = MODULE.matrix_row(
            "C", "client", MODULE.unavailable("scenario_c", "no browser", "CLIENT_E2E_UNAVAILABLE")
        )
        self.assertEqual(rows[2]["status"], MODULE.UNAVAILABLE)
        self.assertEqual(
            MODULE.overall_status(rows, [], [], {"status": MODULE.PASS}), MODULE.FAIL
        )

    def test_missing_scenario_is_an_error(self):
        row = MODULE.matrix_row("B", "render", None)
        self.assertEqual(row["status"], MODULE.ERROR)
        self.assertEqual(row["failure_code"], "MATRIX_SCENARIO_MISSING")

    def test_failed_supplemental_or_preflight_blocks_pass(self):
        fault = {"status": MODULE.PASS}
        self.assertEqual(
            MODULE.overall_status(
                matrix_rows(), [{"status": MODULE.FAIL}], [], fault
            ),
            MODULE.FAIL,
        )
        self.assertEqual(
            MODULE.overall_status(
                matrix_rows(), [], [{"status": MODULE.FAIL}], fault
            ),
            MODULE.FAIL,
        )

    def test_fault_mismatch_is_red_and_matching_fault_is_green(self):
        mismatch = MODULE.fault_verification("test_fault", "CLIENT_DECODE_STALLED", "CLIENT_PRESENTATION_STALLED")
        self.assertEqual(mismatch["status"], MODULE.FAIL)
        match = MODULE.fault_verification("test_fault", "CLIENT_DECODE_STALLED", "CLIENT_DECODE_STALLED")
        self.assertEqual(match["status"], MODULE.PASS)

    def test_default_fault_status_is_unavailable(self):
        self.assertEqual(MODULE.fault_verification(None)["status"], MODULE.UNAVAILABLE)

    def test_render_row_uses_selected_hummingbird_samples(self):
        report = {
            "stage_ready": False,
            "animated_prim_count": 44,
            "server_evidence": {
                "t0": {"stage_ready": True},
                "t1": {"stage_ready": True},
                "hash_t0_differs_from_t1": True,
                "sequence_t1_after_t0": True,
                "sequence_round_trip_after_t1": True,
            },
            "pairwise_render_mad_luma": {
                "hummingbird_t0_t1": 34.0,
                "hummingbird_t0_round_trip": 0.0,
            },
        }
        command = {"exit_code": 0}
        result = MODULE.hummingbird_render_scenario(report, command)
        self.assertEqual(result["status"], MODULE.PASS)


if __name__ == "__main__":
    unittest.main()
