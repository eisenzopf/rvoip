#!/usr/bin/env python3
"""Tests for exact-candidate remote qualification report rendering."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts/release/render_qualification_reports.py"
SPEC = importlib.util.spec_from_file_location("render_qualification_reports", MODULE_PATH)
assert SPEC and SPEC.loader
reports = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reports)


class QualificationReportTests(unittest.TestCase):
    candidate = "a" * 40

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        self.catalog_path = self.root / "catalog.json"
        self.plan_path = self.root / "plan.json"
        self.aggregate_path = self.root / "aggregate.json"
        self.output = self.root / "reports"
        self.catalog = {
            "schema": reports.CATALOG_SCHEMA,
            "gates": [
                {
                    "id": "source.clean",
                    "name": "clean source",
                    "category": "Source integrity",
                    "kind": "source",
                    "executor": "builtin",
                    "resource_class": "github-standard",
                    "display_command": "verify clean source",
                },
                {
                    "id": "test.workspace",
                    "name": "workspace tests",
                    "category": "Build, API, and documentation",
                    "kind": "cargo",
                    "executor": "argv",
                    "resource_class": "github-standard",
                    "display_command": "cargo test --workspace",
                },
            ],
            "profiles": {"remote-release": ["source.clean", "test.workspace"]},
            "remote_release_legacy_coverage": {
                "required_legacy_count": 108,
                "profile_legacy_count": 108,
                "unautomated_legacy_ids": [],
            },
        }
        self.plan = {
            "schema": reports.PLAN_SCHEMA,
            "candidate_sha": self.candidate,
            "profile": "remote-release",
            "gates": [],
        }
        accepted = []
        for index, gate_id in enumerate(("source.clean", "test.workspace"), start=1):
            planned = {
                "id": gate_id,
                "decision": "RUN",
                "gate_definition_sha256": f"{index}" * 64,
                "input_sha256": f"{index + 2}" * 64,
                "environment_sha256": f"{index + 4}" * 64,
            }
            self.plan["gates"].append(planned)
            directory = self.evidence / gate_id
            directory.mkdir()
            log = directory / "command.log"
            log.write_text(f"PASS {gate_id}\n")
            receipt = {
                "schema": reports.RECEIPT_SCHEMA,
                "gate_id": gate_id,
                "candidate_sha": self.candidate,
                "status": "PASS",
                "gate_definition_sha256": planned["gate_definition_sha256"],
                "input_sha256": planned["input_sha256"],
                "environment_id": "test-env",
                "environment_sha256": planned["environment_sha256"],
                "started_at": f"2026-09-05T00:00:0{index}+00:00",
                "ended_at": f"2026-09-05T00:00:1{index}+00:00",
                "duration_seconds": index,
                "attempts": [{"attempt": 1, "argv": ["test"], "exit_code": 0}],
                "log": {
                    "path": log.relative_to(self.evidence).as_posix(),
                    "sha256": reports.sha256_path(log),
                    "bytes": log.stat().st_size,
                },
            }
            (directory / "receipt.json").write_bytes(reports.canonical_bytes(receipt))
            accepted.append(
                {
                    "gate_id": gate_id,
                    "source": "fresh",
                    "receipt_candidate_sha": self.candidate,
                    "receipt_sha256": reports.sha256_bytes(reports.canonical_bytes(receipt)),
                    "input_sha256": planned["input_sha256"],
                }
            )
        perf = self.evidence / "_perf-results/host"
        perf.mkdir(parents=True)
        (perf / "perf_call_setup.json").write_text(
            json.dumps(
                {
                    "scenario": "perf_call_setup",
                    "duration_secs": 30,
                    "environment": {
                        "git_commit": self.candidate,
                        "git_dirty": False,
                        "rvoip_sip_version": "0.3.9",
                    },
                    "load": {"target_cps": 30},
                    "results": {
                        "achieved_cps": 29.5,
                        "asr": 1.0,
                        "calls_offered": 100,
                        "calls_succeeded": 100,
                    },
                    "latency_ns": {"setup_latency": {"p99": 2_000_000}},
                    "resources": {"peak_rss_mb": 64.5},
                }
            )
        )
        self.catalog_path.write_bytes(reports.pretty_bytes(self.catalog))
        self.plan_path.write_bytes(reports.pretty_bytes(self.plan))
        self.aggregate = {
            "schema": reports.AGGREGATE_SCHEMA,
            "candidate_sha": self.candidate,
            "profile": "remote-release",
            "status": "PASS",
            "failures": [],
            "publishing_attempted": False,
            "environment_id": "test-env",
            "generated_at": "2026-09-05T00:01:00+00:00",
            "catalog_sha256": reports.sha256_bytes(reports.canonical_bytes(self.catalog)),
            "gate_count": 2,
            "fresh_count": 2,
            "reused_count": 0,
            "accepted_gates": accepted,
        }
        self.aggregate_path.write_bytes(reports.pretty_bytes(self.aggregate))

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def generate(self) -> None:
        summary, rows, measurements = reports.load_bundle(
            catalog_path=self.catalog_path,
            plan_path=self.plan_path,
            aggregate_path=self.aggregate_path,
            evidence_root=self.evidence,
            version="0.3.9",
        )
        reports.write_bundle(
            output_dir=self.output,
            summary=summary,
            rows=rows,
            measurements=measurements,
            provenance={
                "run_id": "123",
                "run_url": "https://github.com/eisenzopf/rvoip/actions/runs/123",
                "artifact_id": "456",
                "artifact_digest": "sha256:" + "f" * 64,
            },
            generator=MODULE_PATH,
        )

    def test_generates_and_verifies_exact_candidate_reports(self) -> None:
        self.generate()
        reports.verify_bundle(self.output, MODULE_PATH)
        release = (self.output / "BETA_RELEASE_REPORT.md").read_text()
        gates = (self.output / "BETA_GATE_REPORT.md").read_text()
        performance = (self.output / "BETA_PERFORMANCE_REPORT.md").read_text()
        self.assertIn("RVoIP 0.3.9", release)
        self.assertIn("2/2 passed", release)
        self.assertIn(self.candidate, release)
        self.assertIn("108/108 covered", release)
        self.assertIn("test.workspace", gates)
        self.assertIn("29.5", performance)
        self.assertNotIn("0.2.5", release + gates + performance)

    def test_verification_rejects_tampered_report(self) -> None:
        self.generate()
        with (self.output / "BETA_RELEASE_REPORT.md").open("a") as handle:
            handle.write("tampered\n")
        with self.assertRaisesRegex(reports.ReportError, "report file mismatch"):
            reports.verify_bundle(self.output, MODULE_PATH)

    def test_rejects_performance_from_another_commit(self) -> None:
        path = self.evidence / "_perf-results/host/perf_call_setup.json"
        payload = json.loads(path.read_text())
        payload["environment"]["git_commit"] = "b" * 40
        path.write_text(json.dumps(payload))
        with self.assertRaisesRegex(reports.ReportError, "not exact-candidate"):
            reports.load_bundle(
                catalog_path=self.catalog_path,
                plan_path=self.plan_path,
                aggregate_path=self.aggregate_path,
                evidence_root=self.evidence,
                version="0.3.9",
            )

    def test_rejects_missing_gate_receipt(self) -> None:
        (self.evidence / "source.clean/receipt.json").unlink()
        with self.assertRaisesRegex(reports.ReportError, "exactly one exact-candidate receipt"):
            reports.load_bundle(
                catalog_path=self.catalog_path,
                plan_path=self.plan_path,
                aggregate_path=self.aggregate_path,
                evidence_root=self.evidence,
                version="0.3.9",
            )

    def test_accepts_hash_bound_reused_receipt_without_local_log(self) -> None:
        path = self.evidence / "source.clean/receipt.json"
        receipt = json.loads(path.read_text())
        planned = self.plan["gates"][0]
        planned["decision"] = "REUSE"
        planned["reuse_receipt"] = receipt
        planned["reuse_receipt_sha256"] = reports.sha256_bytes(
            reports.canonical_bytes(receipt)
        )
        self.aggregate["accepted_gates"][0]["source"] = "reused"
        self.aggregate["fresh_count"] = 1
        self.aggregate["reused_count"] = 1
        self.plan_path.write_bytes(reports.pretty_bytes(self.plan))
        self.aggregate_path.write_bytes(reports.pretty_bytes(self.aggregate))
        path.unlink()
        summary, rows, _ = reports.load_bundle(
            catalog_path=self.catalog_path,
            plan_path=self.plan_path,
            aggregate_path=self.aggregate_path,
            evidence_root=self.evidence,
            version="0.3.9",
        )
        self.assertEqual(rows[0]["source"], "reused")
        self.assertEqual(rows[0]["command_log_verification"], "collector-accepted-reuse")
        self.assertEqual(summary["qualification"]["fresh_count"], 1)


if __name__ == "__main__":
    unittest.main()
