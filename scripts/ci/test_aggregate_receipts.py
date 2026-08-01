from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("aggregate_receipts.py")
SPEC = importlib.util.spec_from_file_location("aggregate_receipts", SCRIPT)
assert SPEC and SPEC.loader
aggregate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = aggregate
SPEC.loader.exec_module(aggregate)


class AggregateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        self.plan = self.root / "plan.json"
        self.output = self.root / "receipt.json"

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_plan(self, *, shards=None, shard_jobs=None, specialty=None) -> None:
        self.plan.write_text(
            json.dumps(
                {
                    "schema": "rvoip-pr-test-plan-v1",
                    "candidate_sha": "c" * 40,
                    "shards": shards or [],
                    "shard_jobs": shard_jobs if shard_jobs is not None else [],
                    "specialty_gates": specialty or [],
                }
            )
        )

    def write_receipt(self, name: str, status: str = "PASS") -> None:
        (self.evidence / f"{name}.json").write_text(
            json.dumps(
                {
                    "schema": "rvoip-ci-command-receipt-v1",
                    "name": name,
                    "status": status,
                    "git_commit": "c" * 40,
                }
            )
        )

    def invoke(self, *jobs: str) -> int:
        argv = [
            "--plan",
            str(self.plan),
            "--evidence",
            str(self.evidence),
            "--output",
            str(self.output),
        ]
        for job in jobs:
            argv.extend(["--job", job])
        return aggregate.main(argv)

    def test_docs_plan_accepts_skipped_matrix_jobs(self) -> None:
        self.write_plan()
        self.write_receipt("policy")
        self.assertEqual(
            self.invoke(
                "plan=success",
                "policy=success",
                "crate-tests=skipped",
                "specialty=skipped",
            ),
            0,
        )
        self.assertEqual(json.loads(self.output.read_text())["status"], "PASS")

    def test_missing_or_failed_matrix_receipt_fails_gate(self) -> None:
        self.write_plan(
            shards=[{"id": "1"}],
            shard_jobs=[
                {"shard_id": "1", "check": "test"},
                {"shard_id": "1", "check": "clippy"},
            ],
            specialty=["browser-smoke"],
        )
        self.write_receipt("policy")
        self.write_receipt("shard-1-test", "FAIL")
        self.assertEqual(
            self.invoke(
                "plan=success",
                "policy=success",
                "crate-tests=failure",
                "specialty=success",
            ),
            1,
        )
        payload = json.loads(self.output.read_text())
        self.assertEqual(payload["status"], "FAIL")
        self.assertTrue(any("specialty-browser-smoke" in item for item in payload["failures"]))

    def test_receipt_from_different_candidate_fails_gate(self) -> None:
        self.write_plan()
        self.write_receipt("policy")
        (self.evidence / "policy.json").write_text(
            json.dumps(
                {
                    "schema": "rvoip-ci-command-receipt-v1",
                    "name": "policy",
                    "status": "PASS",
                    "git_commit": "d" * 40,
                }
            )
        )
        self.assertEqual(
            self.invoke(
                "plan=success",
                "policy=success",
                "crate-tests=skipped",
                "specialty=skipped",
            ),
            1,
        )
        self.assertIn("instead of", json.loads(self.output.read_text())["failures"][0])


if __name__ == "__main__":
    unittest.main()
