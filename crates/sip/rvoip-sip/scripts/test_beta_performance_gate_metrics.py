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
            "diagnostics": {"media_receive": {"skip_audio_frame_delivery": False}},
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
            "diagnostics": {"media_receive": {"skip_audio_frame_delivery": False}},
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
                "rss_gate_window": "active_tail_1200s",
                "rss_active_tail_window_secs": 1200.0,
                "rss_active_tail_window_complete": True,
                "rss_active_tail_estimator": "theil_sen_pairwise_slopes",
                "rss_post_drain_growth_mb_per_hr": 0.0,
            }
        }
        (self.root / "perf_soak_30min.json").write_text(
            json.dumps(monolithic), encoding="utf-8"
        )
        split_caller = {
            "diagnostics": {"media_receive": {"skip_audio_frame_delivery": False}},
            "results": {
                "duration_secs": 3600,
                "active_calls_target": 500,
                "calls_offered": 9904,
                "calls_succeeded": 9904,
                "asr": 1.0,
                "errors": {
                    "call_failed": 0,
                    "media_setup_failed": 0,
                    "teardown_failed": 0,
                },
                "rss_gate": {"effective_mb_per_hr": 15.0},
                "rss_gate_window": "active_tail_1200s",
                "rss_active_tail_window_secs": 1200.0,
                "rss_active_tail_window_complete": True,
                "rss_gate_growth_mb_per_hr": 5.0,
                "retained_objects_after_drain": 0,
                "transaction_manager_active_after_drain": 0,
                "transaction_runner_active_after_drain": 0,
            },
        }
        split_receiver = {
            "diagnostics": {"media_receive": {"skip_audio_frame_delivery": False}},
            "results": {
                "configured_duration_secs": 3600,
                "active_calls_target": 500,
                "bob_completed_audio_receivers": 9904,
                "bob_received_frames": 80_000_000,
                "bob_active_audio_receivers": 0,
                "rss_gate": {"effective_mb_per_hr": 15.0},
                "rss_gate_window": "active_tail_1200s",
                "rss_active_tail_window_secs": 1200.0,
                "rss_active_tail_window_complete": True,
                "rss_gate_growth_mb_per_hr": 9.0,
                "retained_objects_after_drain": 0,
                "transaction_manager_active_after_drain": 0,
                "transaction_runner_active_after_drain": 0,
                "stop_seen": True,
            },
        }
        (self.root / "perf_soak_caller.json").write_text(
            json.dumps(split_caller), encoding="utf-8"
        )
        (self.root / "perf_soak_receiver.json").write_text(
            json.dumps(split_receiver), encoding="utf-8"
        )
        canonical = self.root / "canonical-2k"
        canonical.mkdir()
        self.candidate_sha = "c" * 40
        self.source_fingerprint = "f" * 64
        self.executable_sha = "e" * 64
        runs = []
        for sequence in range(1, 4):
            run = canonical / f"run-{sequence}"
            run.mkdir()
            report = {
                "scenario": metrics.CANONICAL_SCENARIO,
                "load": {"target_cps": 2000.0},
                "latency_ns": {
                    "setup_latency": {
                        "p50": 11_000_000,
                        "p95": 12_000_000,
                        "p99": 13_000_000,
                    }
                },
                "results": {
                    "achieved_cps": 1850.0 + sequence,
                    "calls_offered": 65_000,
                    "calls_succeeded": 65_000,
                    "asr": 1.0,
                    "errors": {"timeout": 0, "invite_send_failed": 0},
                },
            }
            (run / "report.json").write_text(json.dumps(report), encoding="utf-8")
            runs.append(
                {
                    "sequence": sequence,
                    "packaged_run_dir": f"run-{sequence}",
                    "source_fingerprint_sha256": self.source_fingerprint,
                    "executable_sha256": self.executable_sha,
                }
            )
        self.canonical_index = canonical / "index.json"
        self.canonical_index.write_text(
            json.dumps(
                {
                    "schema": metrics.CANONICAL_SCHEMA,
                    "status": "PASS",
                    "scenario": metrics.CANONICAL_SCENARIO,
                    "run_count": 3,
                    "source_at_beta_start": {
                        "git_commit": self.candidate_sha,
                        "git_dirty": False,
                        "source_fingerprint_sha256": self.source_fingerprint,
                    },
                    "common_source_fingerprint_sha256": self.source_fingerprint,
                    "common_executable_sha256": self.executable_sha,
                    "runs": runs,
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self):
        self.temp.cleanup()

    def test_release_metrics_pass(self):
        canonical = metrics.canonical_2k_metrics(
            self.canonical_index, True, self.candidate_sha
        )
        burst = metrics.high_density_metrics(self.root, 160.0, 0.995, 15.0, True)
        soak = metrics.monolithic_metrics(self.root, 3600, 30, 15.0, True)
        split = metrics.split_soak_metrics(self.root, 3600, 500, 15.0, True)
        self.assertTrue(canonical["passed"])
        self.assertTrue(burst["passed"])
        self.assertTrue(soak["passed"])
        self.assertTrue(split["passed"])
        self.assertIn(
            "full_audio_frame_delivery",
            metrics.markdown(
                {
                    "canonical_2k": canonical,
                    "high_density_media_burst": burst,
                    "monolithic_soak": soak,
                    "split_soak": split,
                }
            ),
        )
        self.assertIn(
            "Canonical 2,000-CPS evaluation",
            metrics.markdown(
                {
                    "canonical_2k": canonical,
                    "high_density_media_burst": burst,
                    "monolithic_soak": soak,
                    "split_soak": split,
                }
            ),
        )

    def test_canonical_requires_three_passing_exact_candidate_runs(self):
        index = json.loads(self.canonical_index.read_text(encoding="utf-8"))
        index["runs"].pop()
        self.canonical_index.write_text(json.dumps(index), encoding="utf-8")
        result = metrics.canonical_2k_metrics(
            self.canonical_index, True, self.candidate_sha
        )
        self.assertFalse(result["passed"])

    def test_canonical_rejects_evidence_from_another_candidate(self):
        result = metrics.canonical_2k_metrics(
            self.canonical_index, True, "d" * 40
        )
        self.assertFalse(result["passed"])

    def test_missing_required_canonical_evidence_fails(self):
        with self.assertRaisesRegex(metrics.MetricsError, "canonical"):
            metrics.canonical_2k_metrics(self.root / "missing.json", True)

    def test_skipped_audio_delivery_fails(self):
        path = next(
            self.root.glob(
                "perf_burst_matrix/burst_*/high-density-media-burst/"
                "perf_burst_receiver_high-density-media-burst.json"
            )
        )
        value = json.loads(path.read_text(encoding="utf-8"))
        value["diagnostics"]["media_receive"]["skip_audio_frame_delivery"] = True
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.high_density_metrics(self.root, 160.0, 0.995, 15.0, True)
        self.assertFalse(result["passed"])

    def test_wrong_rss_limit_fails(self):
        path = self.root / "perf_soak_30min.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["rss_gate"]["effective_mb_per_hr"] = 10.0
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.monolithic_metrics(self.root, 3600, 30, 15.0, True)
        self.assertFalse(result["passed"])

    def test_short_or_wrong_active_tail_window_fails(self):
        path = self.root / "perf_soak_30min.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["rss_gate_window"] = "active_tail_600s"
        value["results"]["rss_active_tail_window_secs"] = 600.0
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.monolithic_metrics(self.root, 3600, 30, 15.0, True)
        self.assertFalse(result["passed"])

    def test_timeout_count_must_reconcile_with_call_accounting(self):
        path = next(
            self.root.glob(
                "perf_burst_matrix/burst_*/high-density-media-burst/"
                "perf_burst_caller_high-density-media-burst.json"
            )
        )
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["errors"]["timeout"] = 100
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.high_density_metrics(self.root, 160.0, 0.995, 15.0, True)
        self.assertFalse(result["passed"])

    def test_split_soak_requires_cross_role_completion_and_drain(self):
        path = self.root / "perf_soak_receiver.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["results"]["bob_completed_audio_receivers"] = 9903
        value["results"]["retained_objects_after_drain"] = 1
        path.write_text(json.dumps(value), encoding="utf-8")
        result = metrics.split_soak_metrics(self.root, 3600, 500, 15.0, True)
        self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
