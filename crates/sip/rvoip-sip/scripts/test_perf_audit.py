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
    def run_audit(
        self,
        mismatched=False,
        incomplete=False,
        with_manifest=False,
        missing_current=False,
    ):
        temporary = tempfile.TemporaryDirectory()
        root = pathlib.Path(temporary.name)
        baseline, current = write_fixture(
            root, mismatched=mismatched, incomplete=incomplete
        )
        if missing_current:
            measured = current / SCENARIO / "2000.json"
            measured.rename(current / SCENARIO / "1000.json")
        output = root / "audit.md"
        arguments = [
            "python3",
            str(SCRIPT),
            "--baseline",
            str(baseline),
            "--current",
            str(current),
            "--out",
            str(output),
            "--fail-on-regression",
        ]
        if with_manifest:
            manifest = root / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "schema": "rvoip-perf-regression-baseline-v1",
                        "baseline_id": "20260929T034251Z",
                        "comparison_paths": [f"{SCENARIO}/2000.json"],
                    }
                ),
                encoding="utf-8",
            )
            arguments.extend(["--baseline-manifest", str(manifest)])
        result = subprocess.run(
            arguments,
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

    def test_reviewed_manifest_identity_is_written_to_report(self):
        result, report = self.run_audit(with_manifest=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("reviewed baseline: `20260929T034251Z`", report)
        self.assertIn("manifest SHA-256", report)

    def test_reviewed_manifest_comparison_path_is_required_in_current(self):
        result, report = self.run_audit(
            with_manifest=True,
            missing_current=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn(
            f"current missing {SCENARIO}/2000.json",
            report,
        )

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



class ReportOnlyLatencyPercentileTests(unittest.TestCase):
    """A report-only percentile is compared and shown but never gates."""

    @staticmethod
    def _audit():
        import importlib.util

        spec = importlib.util.spec_from_file_location("perf_audit_under_test", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    @staticmethod
    def _report(p50, p95, p99):
        return {"latency_ns": {"setup_latency": {"p50": p50, "p95": p95, "p99": p99}}}

    def _rows(self, report_only):
        audit = self._audit()
        base = self._report(2_000_000, 4_800_000, 20_000_000)
        # p99 nearly doubles, as it does at the 2,000-CPS knee on unchanged code;
        # p50 and p95 stay within tolerance.
        cur = self._report(2_100_000, 4_300_000, 39_000_000)
        rows = audit.collect_metrics(base, cur, None, None, 15.0, 50.0, report_only)
        return {label: (regressed, gated) for label, _b, _c, _d, regressed, gated in rows}

    def test_a_knee_p99_swing_gates_by_default(self):
        rows = self._rows(frozenset())
        self.assertEqual(rows["setup_latency p99 (ms)"], (True, True))

    def test_a_report_only_p99_is_shown_but_does_not_gate(self):
        rows = self._rows(frozenset({"p99"}))
        self.assertEqual(rows["setup_latency p99 (ms)"], (False, False))
        # p50 and p95 are still gated, and still pass.
        self.assertEqual(rows["setup_latency p50 (ms)"], (False, True))
        self.assertEqual(rows["setup_latency p95 (ms)"], (False, True))

    def test_a_real_p95_regression_still_fails_with_p99_report_only(self):
        audit = self._audit()
        base = self._report(2_000_000, 4_800_000, 20_000_000)
        cur = self._report(2_100_000, 9_000_000, 21_000_000)
        rows = audit.collect_metrics(base, cur, None, None, 15.0, 50.0, frozenset({"p99"}))
        p95 = next(row for row in rows if row[0] == "setup_latency p95 (ms)")
        self.assertTrue(p95[4], "an 87% p95 rise must still count as a regression")

    def test_an_unknown_percentile_is_refused(self):
        result = subprocess.run(
            [
                "python3", str(SCRIPT),
                "--baseline", "/nonexistent", "--current", "/nonexistent",
                "--out", "/dev/null", "--report-only-latency-percentiles", "p90",
            ],
            capture_output=True, text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown latency percentile", result.stderr)


if __name__ == "__main__":
    unittest.main()
