#!/usr/bin/env python3
"""Fast synthetic tests for the canonical 2,000-CPS acceptance gate."""

import copy
import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("perf_2k_acceptance.py")
SPEC = importlib.util.spec_from_file_location("perf_2k_acceptance", SCRIPT)
acceptance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(acceptance)


def dotted_set(value, path, item):
    parts = path.split(".")
    current = value
    for part in parts[:-1]:
        current = current.setdefault(part, {})
    current[parts[-1]] = copy.deepcopy(item)


def synthetic_resource_samples(cleanup_rate_mb_per_hour=5.0):
    samples = []
    for index in range(0, 1_461):
        t_secs = index * 0.5
        if t_secs <= 35.0:
            # Canonical active load: bounded allocation growth below the
            # reviewed 2,378.44 MB/min ceiling.
            rss_mb = 1_000.0 + 100.0 * t_secs / 35.0
        elif t_secs < 130.0:
            # The 95-second retained-state fence is deliberately excluded from
            # the post-fence RSS gate window.
            rss_mb = 1_100.0 - 100.0 * (t_secs - 35.0) / 95.0
        else:
            rss_mb = 1_000.0 + cleanup_rate_mb_per_hour * (t_secs - 130.0) / 3_600.0
        samples.append(
            {
                "t_secs": t_secs,
                "rss_mb": rss_mb,
                "cpu_pct": 0.0 if index == 0 else 50.0,
            }
        )
    return samples


def refresh_resource_summaries(report):
    resources = report["resources"]
    samples = resources["rss_samples_mb"]
    interval = acceptance._sample_interval_estimate_secs(samples)
    for window in resources["rss_windows"].values():
        window.update(acceptance.recompute_resource_window(samples, window, interval))

    tail_requested = resources["rss_tail_window_requested_secs"]
    tail_min_t = max(0.0, samples[-1]["t_secs"] - tail_requested)
    tail = [sample for sample in samples if sample["t_secs"] >= tail_min_t]
    tail_coverage = acceptance._sample_coverage_secs(tail)
    active = resources["rss_windows"]["active_load"]
    cleanup = resources["rss_windows"]["post_drain_cleanup"]
    resources.update(
        {
            "baseline_rss_mb": samples[0]["rss_mb"],
            "peak_rss_mb": max(sample["rss_mb"] for sample in samples),
            "rss_growth_mb_per_min": acceptance._linear_slope_mb_per_sec(samples)
            * 60.0,
            "rss_tail_growth_mb_per_min": acceptance._linear_slope_mb_per_sec(tail)
            * 60.0,
            "rss_tail_window_secs": tail_coverage,
            "rss_tail_window_complete": tail_coverage + max(interval, 0.001) * 1.5
            >= tail_requested,
            "rss_tail_sample_count": len(tail),
            "rss_sample_interval_estimate_secs": interval,
            "rss_active_growth_mb_per_min": active["rss_growth_mb_per_min"],
            "rss_cleanup_growth_mb_per_min": cleanup["rss_growth_mb_per_min"],
            "rss_cleanup_growth_mb_per_hour": cleanup["rss_growth_mb_per_min"]
            * 60.0,
            "rss_cleanup_retained_growth_mb": cleanup["rss_retained_growth_mb"],
            "rss_cleanup_endpoint_growth_mb_per_hour": cleanup[
                "rss_endpoint_growth_mb_per_hour"
            ],
            "avg_cpu_pct": sum(sample["cpu_pct"] for sample in samples[1:])
            / (len(samples) - 1),
            "rss_sample_count": len(samples),
            "rss_samples_embedded": True,
            "rss_samples_path": None,
        }
    )


def replace_cleanup_rss(report, value_at_elapsed):
    for sample in report["resources"]["rss_samples_mb"]:
        if sample["t_secs"] >= 130.0:
            sample["rss_mb"] = value_at_elapsed(sample["t_secs"] - 130.0)
    refresh_resource_summaries(report)


def canonical_report():
    report = {}
    for path, expected in acceptance.canonical_exact_values():
        dotted_set(report, path, expected)

    dotted_set(
        report,
        "diagnostics.effective_config.runtime_switches.environment.RVOIP_PERF_OUTPUT_ROOT",
        "/tmp/rvoip-canonical-output",
    )
    dotted_set(
        report,
        "diagnostics.phase_markers",
        [
            {"phase": "point_start", "kind": "actual", "elapsed_ms": 0},
            {"phase": "ramp_end", "kind": "planned", "elapsed_ms": 5_000},
            {"phase": "steady_end", "kind": "planned", "elapsed_ms": 35_000},
            {"phase": "cooldown_end", "kind": "planned", "elapsed_ms": 40_000},
            {"phase": "dispatch_complete", "kind": "actual", "elapsed_ms": 35_000},
            {"phase": "calls_drained", "kind": "actual", "elapsed_ms": 35_000},
            {"phase": "post_drain_settle_start", "kind": "actual", "elapsed_ms": 35_000},
            {"phase": "cooldown_end", "kind": "actual", "elapsed_ms": 40_000},
            {"phase": "post_drain_settle_end", "kind": "actual", "elapsed_ms": 130_000},
            {"phase": "post_drain_cleanup_start", "kind": "actual", "elapsed_ms": 130_000},
            {"phase": "post_drain_cleanup_end", "kind": "actual", "elapsed_ms": 730_000},
            {
                "phase": "resource_sampling_stopped",
                "kind": "actual",
                "elapsed_ms": 730_500,
            },
        ],
    )
    report.setdefault("results", {}).update(
        {
            "achieved_cps": 1800.0,
            "asr": 1.0,
            "ner": 1.0,
            "errors": {
                "invite_send_failed": 0,
                "answer_failed": 0,
                "bye_failed": 0,
                "timeout": 0,
                "harness_backpressure_rejected": 0,
                "invite_send_error_counts": {},
            },
        }
    )
    report["latency_ns"] = {
        "setup_latency": {"p50": 11_000_000, "p95": 12_000_000, "p99": 13_000_000},
        "full_cycle": {"p50": 123_000_000, "p95": 125_000_000, "p99": 128_000_000},
    }
    report.setdefault("resources", {}).update(
        {
            "rss_tail_window_requested_secs": 60.0,
            "rss_samples_mb": synthetic_resource_samples(),
            "rss_windows": {
                "active_load": {
                    "name": "active_load",
                    "start_phase": "point_start",
                    "end_phase": "calls_drained",
                    "requested_start_secs": 0.0,
                    "requested_end_secs": 35.0,
                    "requested_coverage_secs": 35.0,
                },
                "post_drain_cleanup": {
                    "name": "post_drain_cleanup",
                    "start_phase": "post_drain_cleanup_start",
                    "end_phase": "post_drain_cleanup_end",
                    "requested_start_secs": 130.0,
                    "requested_end_secs": 730.0,
                    "requested_coverage_secs": 600.0,
                },
            },
        }
    )
    refresh_resource_summaries(report)
    dotted_set(
        report,
        "diagnostics.measurement_identity",
        {
            "schema": "rvoip-sip-perf-measurement-identity-v2",
            "peer_lifecycle": "shared_for_entire_sweep",
            "sweep_points_cps": acceptance.CANONICAL_SWEEP,
            "point_index": 4,
            "measured_point_cps": 2000.0,
            "conditioning": {
                "points": acceptance.CANONICAL_CONDITIONING,
                "point_count": 4,
                "calls_offered": 46_475,
                "calls_succeeded": 46_475,
            },
            "resource_window": {
                "metric": "resources.rss_active_growth_mb_per_min",
                "kind": "active_load",
                "start_phase": "point_start",
                "end_phase": "calls_drained",
                "sample_interval_ms": 500,
            },
            "post_drain_cleanup": {
                "settle_secs": 95,
                "requested_secs": 600,
                "start_phase": "post_drain_cleanup_start",
                "end_phase": "post_drain_cleanup_end",
                "rss_metric": "resources.rss_cleanup_endpoint_growth_mb_per_hour",
                "rss_retained_delta_metric": "resources.rss_cleanup_retained_growth_mb",
                "rss_trend_metric": "resources.rss_cleanup_growth_mb_per_hour",
                "rss_intent_mb_per_hour": 10.0,
                "minimum_representative_separation_secs": 360.0,
                "rss_endpoint_estimator": "median_first_last_sixth_capped_60s",
                "structural_metrics": [
                    "diagnostics.cleanup_convergence_at_settle",
                    "diagnostics.cleanup_convergence",
                ],
            },
        },
    )
    dotted_set(
        report,
        "diagnostics.cleanup_convergence_at_settle",
        {
            "schema": "rvoip-sip-cleanup-convergence-v1",
            "endpoint_count": 5,
            "retained_total": 0,
            "missing_count": 0,
            "converged": True,
        },
    )
    dotted_set(
        report,
        "diagnostics.cleanup_convergence",
        {
            "schema": "rvoip-sip-cleanup-convergence-v1",
            "endpoint_count": 5,
            "retained_total": 0,
            "missing_count": 0,
            "converged": True,
        },
    )

    ranges = ((51_000, 54_633), (54_634, 58_267), (58_268, 61_901), (61_902, 65_535))
    shards = []
    for index, (start, end) in enumerate(ranges):
        shards.append(
            {
                "from": f"sip:alice{index}@127.0.0.1:{40_000 + index}",
                "config": {
                    "incoming_call_channel_capacity": 1000,
                    "state_event_channel_capacity": 1000,
                    "sip_transport_channel_capacity": 10_000,
                    "transaction_event_channel_capacity": 10_000,
                    "effective_sip_transaction_command_channel_capacity": 32,
                    "effective_sip_transaction_dispatch_priority_burst_max": 64,
                    "effective_sip_invite_2xx_retransmit_max_due_per_tick": 2048,
                    "global_event_channel_capacity": 256,
                    "session_event_dispatcher_workers": 16,
                    "server_call_capacity": None,
                    "server_retained_lifecycle_capacity": None,
                    "media_mode": {"kind": "enabled"},
                    "media_port_start": start,
                    "media_port_end": end,
                    "media_port_capacity": 3634,
                    "media_session_capacity": 2000,
                    "auto_180_ringing": True,
                    "auto_100_trying": True,
                    "fast_auto_accept_incoming_calls": False,
                    "diagnostics": {
                        "sip_udp": False,
                        "transaction_timing": False,
                        "dialog_timing": False,
                        "media_setup": False,
                        "cleanup": False,
                    },
                },
            }
        )
    dotted_set(report, "diagnostics.effective_config.alice_shard_configs", shards)
    return report


class AcceptanceTests(unittest.TestCase):
    def evaluate(self, report):
        return acceptance.evaluate(
            report,
            acceptance.CANONICAL_SCENARIO,
            pathlib.Path("synthetic.json"),
        )

    def test_exact_canonical_fixture_passes(self):
        result = self.evaluate(canonical_report())
        self.assertEqual(result["schema"], "rvoip-sip-2k-acceptance-v3")
        self.assertEqual(result["status"], "PASS")

    def test_noncanonical_identity_mutations_fail(self):
        mutations = (
            ("load.steady_secs", 29),
            ("results.calls_offered", 64_999),
            ("environment.global_allocator", "system"),
            ("environment.cargo_features", ["perf-tests", "perf-media-diagnostics"]),
            ("resources.rss_tail_window_secs", 5.0),
            ("diagnostics.measurement_identity.conditioning.points", []),
            ("resources.rss_windows.active_load.end_phase", "cooldown_end"),
            ("resources.rss_windows.post_drain_cleanup.requested_coverage_secs", 600.001947),
            ("resources.rss_windows.post_drain_cleanup.rss_endpoint_separation_secs", 95.0),
            ("diagnostics.cleanup_convergence_at_settle.converged", False),
            ("diagnostics.cleanup_convergence.converged", False),
            ("diagnostics.effective_config.bob.server_retained_lifecycle_capacity", 8000),
            ("diagnostics.effective_config.bob.effective_sip_transaction_dispatch_priority_burst_max", 16),
            ("diagnostics.effective_config.runtime_switches.environment.RVOIP_TEST", "1"),
            ("diagnostics.effective_config.runtime_switches.environment.RVOIP_PERF_OUTPUT_ROOT", "relative"),
        )
        for path, value in mutations:
            with self.subTest(path=path):
                report = canonical_report()
                dotted_set(report, path, value)
                self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_alice_profile_drift_fails(self):
        report = canonical_report()
        report["diagnostics"]["effective_config"]["alice_shard_configs"][2]["config"][
            "session_event_dispatcher_workers"
        ] = 4
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_absolute_metric_regression_fails(self):
        report = canonical_report()
        report["latency_ns"]["setup_latency"]["p99"] = 20_000_000
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

        report = canonical_report()
        report["results"]["achieved_cps"] = float("inf")
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_noisy_long_window_ols_uses_robust_endpoint_gate(self):
        report = canonical_report()
        # An interior allocator plateau makes the diagnostic OLS trend noisy,
        # but it is absent from both 60-second endpoint bands.
        replace_cleanup_rss(
            report,
            lambda elapsed: 1_005.0 if 270.0 <= elapsed < 540.0 else 1_000.0,
        )
        self.assertGreater(
            report["resources"]["rss_cleanup_growth_mb_per_hour"],
            acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR,
        )
        self.assertEqual(
            report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"], 0.0
        )
        self.assertEqual(self.evaluate(report)["status"], "PASS")

    def test_material_cleanup_retention_fails_absolute_gate(self):
        report = canonical_report()
        replace_cleanup_rss(
            report, lambda elapsed: 1_000.0 if elapsed < 70.0 else 1_002.0
        )
        self.assertGreater(
            report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"],
            acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR,
        )
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_monotonic_growth_above_ten_mb_per_hour_fails(self):
        report = canonical_report()
        rate = 10.01
        replace_cleanup_rss(
            report, lambda elapsed: 1_000.0 + rate * elapsed / 3_600.0
        )
        self.assertAlmostEqual(
            report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"], rate
        )
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_outer_window_cannot_relax_endpoint_rate_budget(self):
        report = canonical_report()
        # This retained delta is exactly 10 MB/hour over the outer 600 seconds,
        # but exceeds 10 MB/hour over the endpoint representatives' shorter
        # separation and must not borrow unobserved time.
        retained = (
            acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR
            * acceptance.CLEANUP_RSS_WINDOW_SECS
            / 3_600.0
        )
        replace_cleanup_rss(
            report,
            lambda elapsed: 1_000.0 if elapsed < 540.0 else 1_000.0 + retained,
        )
        rate = report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"]
        self.assertGreater(rate, acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR)
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_legacy_cleanup_slope_without_robust_metric_fails(self):
        report = canonical_report()
        report["resources"].pop("rss_cleanup_retained_growth_mb")
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        cleanup.pop("rss_start_median_mb")
        cleanup.pop("rss_end_median_mb")
        cleanup.pop("rss_retained_growth_mb")
        cleanup.pop("rss_start_representative_secs")
        cleanup.pop("rss_end_representative_secs")
        cleanup.pop("rss_endpoint_separation_secs")
        cleanup.pop("rss_endpoint_growth_mb_per_hour")
        report["resources"].pop("rss_cleanup_endpoint_growth_mb_per_hour")
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_raw_sample_mutation_without_summary_update_fails(self):
        report = canonical_report()
        report["resources"]["rss_samples_mb"][900]["rss_mb"] += 100.0
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_missing_or_truncated_raw_samples_fail(self):
        report = canonical_report()
        report["resources"]["rss_samples_mb"] = []
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

        report = canonical_report()
        report["resources"]["rss_samples_mb"].pop()
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_unordered_or_nonfinite_raw_samples_fail(self):
        report = canonical_report()
        report["resources"]["rss_samples_mb"][400]["t_secs"] = report["resources"][
            "rss_samples_mb"
        ][399]["t_secs"]
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

        report = canonical_report()
        report["resources"]["rss_samples_mb"][400]["rss_mb"] = float("nan")
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_summary_scalar_mutation_fails_raw_recomputation(self):
        report = canonical_report()
        report["resources"]["rss_cleanup_growth_mb_per_hour"] += 1.0
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_structural_retention_fails_even_with_falling_rss(self):
        report = canonical_report()
        replace_cleanup_rss(report, lambda elapsed: 1_001.0 - elapsed / 600.0)
        report["diagnostics"]["cleanup_convergence_at_settle"].update(
            retained_total=1, converged=False
        )
        self.assertLess(
            report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"], 0.0
        )
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_short_cleanup_coverage_fails_power_budget(self):
        report = canonical_report()
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        cleanup["requested_end_secs"] = 225.0
        cleanup["requested_coverage_secs"] = 95.0
        refresh_resource_summaries(report)
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_cleanup_window_cannot_move_into_settle_phase(self):
        report = canonical_report()
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        cleanup["requested_start_secs"] = 40.0
        cleanup["requested_end_secs"] = 640.0
        refresh_resource_summaries(report)
        self.assertLess(
            report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"], 0.0
        )
        result = self.evaluate(report)
        self.assertEqual(result["status"], "FAIL")
        failed_metrics = {
            check["metric"] for check in result["checks"] if not check["passed"]
        }
        self.assertIn(
            "resources.rss_windows.phase_marker_binding", failed_metrics
        )

    def test_bounded_sampler_stop_overshoot_preserves_declared_coverage(self):
        report = canonical_report()
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        cleanup["requested_end_secs"] = 730.2
        for marker in report["diagnostics"]["phase_markers"]:
            if marker.get("phase") == "post_drain_cleanup_end":
                marker["elapsed_ms"] = 730_200
        refresh_resource_summaries(report)
        self.assertEqual(cleanup["requested_coverage_secs"], 600.0)
        self.assertEqual(self.evaluate(report)["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
