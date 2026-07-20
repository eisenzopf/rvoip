#!/usr/bin/env python3
"""Synthetic tests for conditioning/window-aware perf comparisons."""

import copy
import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("perf_audit.py")
POINTS = [30.0, 100.0, 300.0, 1000.0, 2000.0]
CALLS = [975, 3250, 9750, 32500, 65000]
SCENARIO = "perf_call_setup_cps_pbx-media-server"


def point_report(point, calls):
    return {
        "scenario": SCENARIO,
        "environment": {"git_rev": "synthetic"},
        "load": {"target_cps": point},
        "results": {
            "achieved_cps": point * 0.92,
            "cps_per_core": point / 10,
            "asr": 1.0,
            "ner": 1.0,
            "calls_offered": calls,
            "calls_succeeded": calls,
        },
        "latency_ns": {"setup_latency": {"p50": 1_000_000, "p95": 2_000_000, "p99": 3_000_000}},
        "resources": {
            "peak_rss_mb": 1000.0,
            "rss_tail_growth_mb_per_min": 100.0,
            "rss_tail_window_secs": 60.0,
            "rss_sample_count": 72,
            "avg_cpu_pct": 100.0,
        },
    }


def explicit_identity(conditioning):
    return {
        "schema": "rvoip-sip-perf-measurement-identity-v1",
        "peer_lifecycle": "shared_for_entire_sweep",
        "sweep_points_cps": POINTS,
        "point_index": 4,
        "measured_point_cps": 2000.0,
        "conditioning": {"points": conditioning},
        "resource_window": {
            "kind": "active_load",
            "start_phase": "point_start",
            "end_phase": "calls_drained",
            "sample_interval_ms": 500,
        },
    }


def write_fixture(root, mismatched=False, incomplete=False):
    baseline = root / "baseline" / SCENARIO
    current = root / "current" / SCENARIO
    baseline.mkdir(parents=True)
    current.mkdir(parents=True)
    for point, calls in zip(POINTS, CALLS):
        (baseline / f"{point:g}.json").write_text(json.dumps(point_report(point, calls)))
    (baseline / "_sweep.json").write_text(
        json.dumps({"sweep_summary": {"points": POINTS}})
    )

    measured = copy.deepcopy(point_report(2000.0, 65000))
    conditioning = [
        {"target_cps": point, "calls_offered": calls, "calls_succeeded": calls}
        for point, calls in zip(POINTS[:-1], CALLS[:-1])
    ]
    if mismatched:
        conditioning[-1]["calls_succeeded"] -= 1
    measured["diagnostics"] = {
        "measurement_identity": explicit_identity(conditioning)
    }
    measured["resources"]["rss_active_growth_mb_per_min"] = 100.0
    measured["resources"]["rss_windows"] = {
        "active_load": {
            "complete": not incomplete,
            "sample_count": 71,
            "actual_coverage_secs": 35.0,
        }
    }
    measured["resources"]["rss_tail_window_requested_secs"] = 60.0
    measured["resources"]["rss_tail_window_secs"] = 35.0
    (current / "2000.json").write_text(json.dumps(measured))
    return root / "baseline", root / "current"


class PerfAuditIdentityTests(unittest.TestCase):
    def run_audit(self, mismatched=False, incomplete=False):
        temporary = tempfile.TemporaryDirectory()
        root = pathlib.Path(temporary.name)
        baseline, current = write_fixture(
            root, mismatched=mismatched, incomplete=incomplete
        )
        output = root / "audit.md"
        result = subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--baseline",
                str(baseline),
                "--current",
                str(current),
                "--out",
                str(output),
                "--fail-on-regression",
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        report = output.read_text()
        temporary.cleanup()
        return result, report

    def test_complete_legacy_sweep_matches_explicit_identity(self):
        result, report = self.run_audit()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("status: OK", report)
        self.assertIn("legacy_complete_sweep_inference", report)
        self.assertIn("RSS active-load growth", report)

    def test_conditioning_difference_is_refused_not_compared(self):
        result, report = self.run_audit(mismatched=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("status: NON_COMPARABLE", report)
        self.assertIn("No scalar comparison was performed", report)
        self.assertIn("NON_COMPARABLE", result.stderr)

    def test_incomplete_explicit_window_is_refused_not_compared(self):
        result, report = self.run_audit(incomplete=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("status: NON_COMPARABLE", report)
        self.assertIn("active-load resource window is incomplete", report)


if __name__ == "__main__":
    unittest.main()
