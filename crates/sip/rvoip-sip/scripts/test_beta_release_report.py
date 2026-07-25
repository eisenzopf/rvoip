#!/usr/bin/env python3
"""Tests for the evidence-complete beta release report generator."""

from __future__ import annotations

import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
CRATE_DIR = SCRIPT_DIR.parent
POLICY_PATH = CRATE_DIR / "config/beta-release-policy.yaml"
MODULE_SPEC = importlib.util.spec_from_file_location(
    "beta_release_report", SCRIPT_DIR / "beta_release_report.py"
)
assert MODULE_SPEC and MODULE_SPEC.loader
reporting = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(reporting)


def available_current_report() -> Path | None:
    configured = os.environ.get("RVOIP_BETA_REPORT_ROOT")
    candidates = [
        Path(configured) if configured else None,
        Path(
            "/Users/jonathan/Developer/rvoip-beta-evidence/"
            "20260724T231330Z/reports/20260724T231400Z"
        ),
    ]
    return next((path for path in candidates if path and (path / "attestation.json").is_file()), None)


class PolicyTests(unittest.TestCase):
    def test_catalog_is_unique_reachable_and_current_full_selects_108(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        gates = reporting.expand_catalog(policy)
        self.assertEqual(len({gate["id"] for gate in gates}), len(gates))
        self.assertEqual(len({gate["name"] for gate in gates}), len(gates))
        config = {
            key: definition.get("full_default", definition.get("default"))
            for key, definition in policy["configuration"].items()
        }
        config.update(
            {
                "beta_gate_mode": "full",
                "beta_require_canonical_2k_evidence": True,
                "beta_run_local_pbx": True,
                "beta_restore_local_pbx": True,
                "beta_run_sipp": True,
                "beta_run_strict_ua": True,
                "beta_run_perf_all": True,
                "beta_run_burst_smoke": True,
                "beta_run_burst_matrix": True,
                "beta_run_long_soak": True,
            }
        )
        selected = [
            gate
            for gate in gates
            if reporting.gate_selected(gate, "full", config)
        ]
        self.assertEqual(len(selected), 108)

    def test_every_condition_and_validator_is_defined(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        definitions = policy["configuration"]
        for gate in reporting.expand_catalog(policy):
            self.assertTrue(set(gate["validators"]) <= reporting.KNOWN_VALIDATORS)
            for operator in ("when", "unless"):
                key = gate["condition"].get(operator)
                if key:
                    self.assertIn(key, definitions)

    def test_typed_configuration_and_safe_paths_fail_closed(self) -> None:
        self.assertIs(
            reporting.convert_typed("0", {"type": "boolean"}), False
        )
        self.assertEqual(
            reporting.convert_typed("30 100,300", {"type": "string-list"}),
            ["30", "100", "300"],
        )
        with self.assertRaises(reporting.ReportError):
            reporting.convert_typed("sometimes", {"type": "boolean"})
        self.assertTrue(reporting.safe_relative("perf-results/result.json"))
        self.assertFalse(reporting.safe_relative("../outside.json"))
        self.assertFalse(reporting.safe_relative("/absolute/path"))

    def test_native_gate_fragments_are_typed_and_deduplicated(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = type(
                "Args",
                (),
                {
                    "policy": POLICY_PATH,
                    "results_dir": root / "parts",
                    "sequence": 1,
                    "name": "format check",
                    "status": "PASS",
                    "started": "2026-07-25T00:00:00Z",
                    "ended": "2026-07-25T00:00:01Z",
                    "duration": 1,
                    "exit_status": 0,
                    "log": "format_check.log",
                    "log_sha256": "a" * 64,
                    "argv": ["cargo", "fmt", "--check"],
                },
            )()
            reporting.record_gate(args)
            output = root / "gate-results.json"
            reporting.finalize_gates(args.results_dir, output, "full")
            payload = json.loads(output.read_text())
            self.assertEqual(payload["schema"], reporting.SCHEMA_RESULTS)
            self.assertEqual(payload["passed"], 1)
            self.assertEqual(payload["records"][0]["id"], "build.format")
            self.assertEqual(
                payload["records"][0]["sanitized_argv"],
                ["cargo", "fmt", "--check"],
            )


@unittest.skipUnless(available_current_report(), "current verified beta report is unavailable")
class CurrentCandidateIntegrationTests(unittest.TestCase):
    def setUp(self) -> None:
        report = available_current_report()
        assert report
        self.report_root = report

    def test_current_package_maps_exactly_108_required_gates(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        attestation = reporting.validate_source_attestation(self.report_root, policy)
        effective = reporting.effective_configuration(
            self.report_root, attestation, policy
        )
        self.assertFalse(
            [
                item["key"]
                for item in effective["values"]
                if item["key"] not in policy["configuration"]
            ],
            "every effective candidate value must have a typed policy definition",
        )
        gates = reporting.build_gate_results(
            self.report_root, attestation, policy, effective
        )
        self.assertEqual(
            (gates["required_count"], gates["passed"], gates["failed"], gates["skipped"]),
            (108, 108, 0, 0),
        )
        self.assertEqual(
            sum(
                gate["evidence_strength"].startswith("legacy-v1")
                for gate in gates["records"]
            ),
            2,
        )

    def test_generation_is_deterministic_and_tamper_evident(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            first_path = Path(first)
            second_path = Path(second)
            reporting.generate(self.report_root, POLICY_PATH, first_path)
            reporting.generate(self.report_root, POLICY_PATH, second_path)
            for name in reporting.ALL_GENERATED_FILES:
                self.assertEqual(
                    (first_path / name).read_bytes(),
                    (second_path / name).read_bytes(),
                    name,
                )
            reporting.verify_generated(first_path, POLICY_PATH)
            release = first_path / "BETA_RELEASE_REPORT.md"
            release.write_text(release.read_text() + "\ntampered\n")
            with self.assertRaises(reporting.ReportError):
                reporting.verify_generated(first_path, POLICY_PATH)

    def test_generated_outputs_have_no_user_absolute_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            reporting.generate(self.report_root, POLICY_PATH, output)
            for name in reporting.REPORT_FILES + reporting.MACHINE_FILES:
                text = (output / name).read_text()
                self.assertNotIn("/Users/", text, name)


if __name__ == "__main__":
    unittest.main()
