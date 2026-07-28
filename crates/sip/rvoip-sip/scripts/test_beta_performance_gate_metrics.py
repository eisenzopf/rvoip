import importlib.util
import json
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("beta_performance_gate_metrics.py")
SPEC = importlib.util.spec_from_file_location("beta_performance_gate_metrics", SCRIPT)
metrics = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(metrics)


class BetaPerformanceGateMetricsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        burst = self.root / "perf_burst_matrix/burst_fixture/high-density-media-burst"
        burst.mkdir(parents=True)
        definition = {
            "phases": [{}, {"cps": 160.0}],
            "acceptance": {
                "minAsr": 0.995,
                "maxRssGrowthMbPerHr": 15.0,
            },
        }
        caller = {
            "diagnostics": {
                "media_receive": {"skip_audio_frame_delivery": False}
            },
            "results": {
                "scenario_definition": definition,
                "asr": 0.998,
                "calls_offered": 18000,
                "calls_succeeded": 17964,
                "calls_failed": 36,
                "errors": {
                    "answer_failed": 0,
                    "invite_send_failed": 0,
                    "media_setup_failed": 0,
                    "overload_rejected": 0,
                    "teardown_failed": 0,
                    "timeout": 36,
                },
                "active_call_occupancy": {
                    "peak_active_calls": 9990,
                    "peak_pending_setups": 800,
                },
                "retained_objects_after_drain": 0,
                "transaction_manager_active_after_drain": 0,
                "rss_gate_growth_mb_per_hr": 0.0,
            },
        }
        receiver = {
            "diagnostics": {
                "media_receive": {"skip_audio_frame_delivery": False}
            },
            "results": {
                "scenario_definition": definition,
                "retained_objects_after_drain": 0,
                "transaction_manager_active_after_drain": 0,
                "bob_active_audio_receivers": 0,
                "bob_completed_audio_receivers": 17980,
                "bob_received_frames": 10_000_000,
                "rss_gate_growth_mb_per_hr": 0.0,
            },
        }
        (burst / "perf_burst_caller_high-density-media-burst.json").write_text(
            json.dumps(caller), encoding="utf-8"
        )
        (burst / "perf_burst_receiver_high-density-media-burst.json").write_text(
            json.dumps(receiver), encoding="utf-8"
        )
        monolithic = {
            "results": {
                "duration_secs": 3600,
                "active_calls_target": 30,
                "rss_gate": {"effective_mb_per_hr": 15.0},
                "errors": {
                    "answer_failed": 0,
                    "answer_timeout": 0,
                    "call_failed": 0,
                    "invite_send_failed": 0,
                    "media_setup_failed": 0,
                    "teardown_failed": 0,
                },
                "retained_objects_after_drain": 0,
                "bob_active_audio_receivers": 0,
                "transaction_manager_active_after_drain": 0,
                "transaction_runner_active_after_drain": 0,
                "controlled_drain_failed": 0,
                "calls_offered": 587,
                "calls_succeeded": 587,
                "bob_received_frames": 1_000_000,
                "rss_gate_growth_mb_per_hr": 12.55,
                "rss_gate_window": "active_tail_600s",
                "rss_active_tail_window_secs": 600.0,
                "rss_post_drain_growth_mb_per_hr": 0.0,
            }
        }
        (self.root / "perf_soak_30min.json").write_text(
            json.dumps(monolithic), encoding="utf-8"
        )

    def tearDown(self):
        self.temp.cleanup()

    def test_release_metrics_pass(self):
        burst = metrics.high_density_metrics(self.root, 160.0, 0.995, 15.0, True)
        soak = metrics.monolithic_metrics(self.root, 3600, 30, 15.0, True)
        self.assertTrue(burst["passed"])
        self.assertTrue(soak["passed"])
        self.assertIn("full_audio_frame_delivery", metrics.markdown({
            "high_density_media_burst": burst,
            "monolithic_soak": soak,
        }))

    def test_skipped_audio_delivery_fails(self):
        path = next(self.root.glob(
            "perf_burst_matrix/burst_*/high-density-media-burst/"
            "perf_burst_receiver_high-density-media-burst.json"
        ))
        value = json.loads(path.read_text(encoding="utf-8"))
        value["diagnostics"]["media_receive"]["skip_audio_frame_delivery"] = True
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.high_density_metrics(
            self.root, 160.0, 0.995, 15.0, True
        )
        self.assertFalse(result["passed"])

    def test_wrong_rss_limit_fails(self):
        path = self.root / "perf_soak_30min.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["rss_gate"]["effective_mb_per_hr"] = 10.0
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.monolithic_metrics(self.root, 3600, 30, 15.0, True)
        self.assertFalse(result["passed"])

    def test_timeout_count_must_reconcile_with_call_accounting(self):
        path = next(self.root.glob(
            "perf_burst_matrix/burst_*/high-density-media-burst/"
            "perf_burst_caller_high-density-media-burst.json"
        ))
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["errors"]["timeout"] = 100
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.high_density_metrics(
            self.root, 160.0, 0.995, 15.0, True
        )
        self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
