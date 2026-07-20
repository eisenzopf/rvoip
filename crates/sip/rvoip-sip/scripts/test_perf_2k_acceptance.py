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
            {"phase": "dispatch_complete", "kind": "actual", "elapsed_ms": 35_001},
            {"phase": "calls_drained", "kind": "actual", "elapsed_ms": 35_200},
            {"phase": "post_drain_cleanup_start", "kind": "actual", "elapsed_ms": 35_200},
            {"phase": "cooldown_end", "kind": "actual", "elapsed_ms": 40_000},
            {"phase": "post_drain_cleanup_end", "kind": "actual", "elapsed_ms": 130_200},
            {
                "phase": "resource_sampling_stopped",
                "kind": "actual",
                "elapsed_ms": 130_500,
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
            "peak_rss_mb": 2800.0,
            "rss_tail_growth_mb_per_min": 0.0,
            "rss_tail_window_secs": 59.5,
            "rss_tail_window_complete": True,
            "rss_tail_sample_count": 120,
            "rss_active_growth_mb_per_min": 2000.0,
            # The short-window trend remains diagnostic. The gate uses the
            # robust absolute endpoint delta below.
            "rss_cleanup_growth_mb_per_min": 0.6606666667,
            "rss_cleanup_growth_mb_per_hour": 39.64,
            "rss_cleanup_retained_growth_mb": 0.2,
            "rss_cleanup_endpoint_growth_mb_per_hour": 0.2 * 3600.0 / 79.5,
            "rss_windows": {
                "active_load": {
                    "name": "active_load",
                    "start_phase": "point_start",
                    "end_phase": "calls_drained",
                    "requested_coverage_secs": 35.2,
                    "actual_coverage_secs": 35.0,
                    "sample_count": 71,
                    "complete": True,
                },
                "post_drain_cleanup": {
                    "name": "post_drain_cleanup",
                    "start_phase": "calls_drained",
                    "end_phase": "post_drain_cleanup_end",
                    "requested_coverage_secs": 95.0,
                    "first_sample_secs": 35.5,
                    "last_sample_secs": 130.0,
                    "actual_coverage_secs": 94.5,
                    "sample_count": 190,
                    "complete": True,
                    "rss_start_median_mb": 4500.0,
                    "rss_end_median_mb": 4500.2,
                    "rss_retained_growth_mb": 0.2,
                    "rss_start_representative_secs": 43.0,
                    "rss_end_representative_secs": 122.5,
                    "rss_endpoint_separation_secs": 79.5,
                    "rss_endpoint_growth_mb_per_hour": 0.2 * 3600.0 / 79.5,
                    "rss_endpoint_band_secs": 15.0,
                    "rss_start_sample_count": 30,
                    "rss_end_sample_count": 30,
                },
            },
        }
    )
    dotted_set(
        report,
        "diagnostics.measurement_identity",
        {
            "schema": "rvoip-sip-perf-measurement-identity-v1",
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
                "requested_secs": 95,
                "rss_metric": "resources.rss_cleanup_endpoint_growth_mb_per_hour",
                "rss_retained_delta_metric": "resources.rss_cleanup_retained_growth_mb",
                "rss_trend_metric": "resources.rss_cleanup_growth_mb_per_hour",
                "rss_intent_mb_per_hour": 10.0,
                "rss_endpoint_estimator": "median_first_last_sixth_capped_15s",
                "structural_metric": "diagnostics.cleanup_convergence.converged",
            },
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
        self.assertEqual(self.evaluate(canonical_report())["status"], "PASS")

    def test_noncanonical_identity_mutations_fail(self):
        mutations = (
            ("load.steady_secs", 29),
            ("results.calls_offered", 64_999),
            ("environment.global_allocator", "system"),
            ("environment.cargo_features", ["perf-tests", "perf-media-diagnostics"]),
            ("resources.rss_tail_window_secs", 5.0),
            ("diagnostics.measurement_identity.conditioning.points", []),
            ("resources.rss_windows.active_load.end_phase", "cooldown_end"),
            ("resources.rss_windows.post_drain_cleanup.requested_coverage_secs", 95.001947),
            ("resources.rss_windows.post_drain_cleanup.rss_endpoint_separation_secs", 95.0),
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

    def test_noisy_short_window_slope_uses_robust_absolute_gate(self):
        report = canonical_report()
        report["resources"]["rss_cleanup_growth_mb_per_hour"] = 39.64
        report["resources"]["rss_cleanup_retained_growth_mb"] = 0.2
        report["resources"]["rss_windows"]["post_drain_cleanup"][
            "rss_retained_growth_mb"
        ] = 0.2
        self.assertEqual(self.evaluate(report)["status"], "PASS")

    def test_material_cleanup_retention_fails_absolute_gate(self):
        report = canonical_report()
        separation = report["resources"]["rss_windows"]["post_drain_cleanup"][
            "rss_endpoint_separation_secs"
        ]
        retained = acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR * separation / 3600.0 + 0.01
        report["resources"]["rss_cleanup_retained_growth_mb"] = retained
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        cleanup["rss_end_median_mb"] = cleanup["rss_start_median_mb"] + retained
        cleanup["rss_retained_growth_mb"] = retained
        rate = retained * 3600.0 / separation
        cleanup["rss_endpoint_growth_mb_per_hour"] = rate
        report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"] = rate
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_monotonic_growth_above_ten_mb_per_hour_fails(self):
        report = canonical_report()
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        separation = cleanup["rss_endpoint_separation_secs"]
        rate = 10.01
        retained = rate * separation / 3600.0
        cleanup["rss_end_median_mb"] = cleanup["rss_start_median_mb"] + retained
        cleanup["rss_retained_growth_mb"] = retained
        cleanup["rss_endpoint_growth_mb_per_hour"] = rate
        report["resources"]["rss_cleanup_retained_growth_mb"] = retained
        report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"] = rate
        self.assertEqual(self.evaluate(report)["status"], "FAIL")

    def test_outer_window_cannot_relax_endpoint_rate_budget(self):
        report = canonical_report()
        cleanup = report["resources"]["rss_windows"]["post_drain_cleanup"]
        # This delta is exactly 10 MB/hour over the outer 95-second window, but
        # exceeds 10 MB/hour over the actual 79.5-second representative span.
        retained = acceptance.CLEANUP_RSS_INTENT_MB_PER_HOUR * 95.0 / 3600.0
        cleanup["rss_end_median_mb"] = cleanup["rss_start_median_mb"] + retained
        cleanup["rss_retained_growth_mb"] = retained
        rate = retained * 3600.0 / cleanup["rss_endpoint_separation_secs"]
        cleanup["rss_endpoint_growth_mb_per_hour"] = rate
        report["resources"]["rss_cleanup_retained_growth_mb"] = retained
        report["resources"]["rss_cleanup_endpoint_growth_mb_per_hour"] = rate
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


if __name__ == "__main__":
    unittest.main()
