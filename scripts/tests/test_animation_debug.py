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

    def test_unrequested_fault_check_is_informational(self):
        fault = MODULE.fault_verification(None)
        self.assertEqual(fault["status"], MODULE.WARN)
        self.assertEqual(
            MODULE.overall_status(matrix_rows(), [], [], fault), MODULE.PASS
        )

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

    def test_client_row_accepts_labelled_fallback_delivery_proof(self):
        report = {
            "client_samples": [
                {"frames_received": 10, "delivery_frames": 8},
                {"frames_received": 20, "delivery_frames": 18},
            ],
            "client_evidence": {
                "decoded_delta": 10,
                "presented_delta": None,
                "proofs": ["rtp_decoded"],
            },
        }
        result = MODULE.client_animation_scenario(
            report,
            {"exit_code": 0},
            {
                "status": MODULE.PASS,
                "frontend_server_started": True,
                "tauri_started": True,
                "backend_started": True,
            },
            {"run_id": "test", "scenario_code": "S12"},
        )
        self.assertEqual(result["status"], MODULE.PASS)
        self.assertEqual(
            result["evidence"]["presentation_source"], "fallback_delivery_or_decode"
        )

    def test_client_row_rejects_missing_real_runtime_evidence(self):
        result = MODULE.client_animation_scenario(
            {"client_samples": [], "client_evidence": {}},
            {"exit_code": 0},
            {
                "status": MODULE.PASS,
                "frontend_server_started": True,
                "tauri_started": True,
                "backend_started": True,
            },
            None,
        )
        self.assertEqual(result["status"], MODULE.FAIL)
        self.assertEqual(result["failure_code"], "ENCODE_OR_WEBRTC_STALLED")


if __name__ == "__main__":
    unittest.main()
