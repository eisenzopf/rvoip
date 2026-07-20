#!/usr/bin/env python3
"""Fast fixture tests for canonical 2,000-CPS beta evidence import."""

import hashlib
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


evidence = load_module(
    "test_canonical_2k_evidence_impl", SCRIPT_DIR / "canonical_2k_evidence.py"
)
acceptance_fixture = load_module(
    "test_canonical_2k_acceptance_fixture",
    SCRIPT_DIR / "test_perf_2k_acceptance.py",
)


class CanonicalEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        (self.workspace / ".gitignore").write_text("/target\n", encoding="utf-8")
        (self.workspace / "source.txt").write_text("release source\n", encoding="utf-8")
        subprocess.run(["git", "init", "-q"], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "config", "user.email", "fixture@example.invalid"],
            cwd=self.workspace,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Fixture"],
            cwd=self.workspace,
            check=True,
        )
        subprocess.run(["git", "add", "."], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "fixture"], cwd=self.workspace, check=True
        )
        self.source = evidence.capture_source_provenance(self.workspace)
        self.fingerprint = self.source["source_fingerprint_sha256"]
        self.beta_start = self.root / "source-at-beta-start.json"
        self.beta_start.write_text(
            json.dumps(self.source, indent=2) + "\n", encoding="utf-8"
        )
        self.binary = self.workspace / "target" / "fixture-bin"
        self.binary.parent.mkdir()
        self.binary.write_bytes(b"exact canonical executable")
        self.binary.chmod(0o755)
        self.binary_sha256 = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.runs = [self.make_run(index) for index in range(1, 4)]

    def tearDown(self):
        self.temp.cleanup()

    def make_run(self, sequence):
        run_dir = self.root / f"run-{sequence}"
        run_dir.mkdir()
        report = acceptance_fixture.canonical_report()
        report.setdefault("environment", {}).update(
            {
                "source_fingerprint_sha256": self.fingerprint,
                "git_commit": self.source["git_commit"],
                "git_rev": self.source["git_rev"],
                "git_dirty": self.source["git_dirty"],
            }
        )
        report_path = run_dir / "report.json"
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        acceptance = evidence.ACCEPTANCE.evaluate(
            report, evidence.CANONICAL_SCENARIO, report_path
        )
        self.assertEqual(acceptance["status"], "PASS")
        (run_dir / "acceptance.json").write_text(
            json.dumps(acceptance, indent=2) + "\n", encoding="utf-8"
        )
        (run_dir / "perf-audit.md").write_text(
            "# Perf Regression Audit\n\nstatus: OK\n", encoding="utf-8"
        )
        source_block = dict(self.source)
        manifest = {
            "schema": evidence.MANIFEST_SCHEMA,
            "captured_at_utc": f"2026-07-18T20:0{sequence}:00+00:00",
            "mode": "clean",
            "scenario": evidence.CANONICAL_SCENARIO,
            "status": 0,
            "overall_status": "PASS",
            "run_executed": True,
            "test_exit_code": 0,
            "report_status": "CAPTURED",
            "acceptance_status": "PASS",
            "perf_audit_status": "PASS",
            "perf_audit_exit_code": 0,
            "source_at_build": source_block,
            "source_at_finalize": source_block,
            "environment": report["environment"],
            "source_fingerprint_matches_runtime": True,
            "source_fingerprint_matches_finalize": True,
            "source_fingerprint_unchanged_for_full_run": True,
            "executable": str(self.binary),
            "executable_sha256": self.binary_sha256,
        }
        (run_dir / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        return run_dir

    def rewrite_manifest(self, run_dir, update):
        path = run_dir / "manifest.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        update(value)
        path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def rewrite_acceptance(self, run_dir, update):
        path = run_dir / "acceptance.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        update(value)
        path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def test_three_passes_are_validated_copied_and_indexed(self):
        artifact_dir = self.root / "artifacts"
        destination = evidence.import_evidence(
            self.workspace, self.beta_start, artifact_dir, self.runs
        )
        index = json.loads((destination / "index.json").read_text(encoding="utf-8"))
        self.assertEqual(index["schema"], evidence.INDEX_SCHEMA)
        self.assertEqual(index["status"], "PASS")
        self.assertEqual(index["run_count"], 3)
        self.assertEqual(index["common_source_fingerprint_sha256"], self.fingerprint)
        self.assertEqual(index["common_executable_sha256"], self.binary_sha256)
        packaged_executable = destination / index["packaged_executable"]
        self.assertTrue(packaged_executable.is_file())
        self.assertEqual(evidence.file_sha256(packaged_executable), self.binary_sha256)
        for sequence in range(1, 4):
            copied = destination / f"run-{sequence}" / "manifest.json"
            self.assertTrue(copied.is_file())
            self.assertEqual(copied.stat().st_mode & 0o222, 0)

    def test_nonpass_manifest_is_rejected(self):
        self.rewrite_manifest(
            self.runs[1], lambda manifest: manifest.update(overall_status="FAIL")
        )
        with self.assertRaisesRegex(evidence.EvidenceError, "overall_status"):
            evidence.import_evidence(
                self.workspace, self.beta_start, self.root / "artifacts", self.runs
            )

    def test_stale_persisted_acceptance_schema_is_rejected(self):
        self.rewrite_acceptance(
            self.runs[0],
            lambda acceptance: acceptance.update(
                schema="rvoip-sip-2k-acceptance-v2"
            ),
        )
        with self.assertRaisesRegex(evidence.EvidenceError, "acceptance.schema"):
            evidence.import_evidence(
                self.workspace, self.beta_start, self.root / "artifacts", self.runs
            )

    def test_unknown_fingerprint_is_rejected(self):
        def make_unknown(manifest):
            manifest["source_at_finalize"]["source_fingerprint_sha256"] = "unknown"

        self.rewrite_manifest(self.runs[2], make_unknown)
        with self.assertRaisesRegex(evidence.EvidenceError, "invalid or unknown"):
            evidence.import_evidence(
                self.workspace, self.beta_start, self.root / "artifacts", self.runs
            )

    def test_source_mutation_after_beta_start_is_rejected(self):
        (self.workspace / "source.txt").write_text("mutated source\n", encoding="utf-8")
        with self.assertRaisesRegex(evidence.EvidenceError, "changed after beta-start"):
            evidence.import_evidence(
                self.workspace, self.beta_start, self.root / "artifacts", self.runs
            )

    def test_run_order_must_be_chronological(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "chronological"):
            evidence.import_evidence(
                self.workspace,
                self.beta_start,
                self.root / "artifacts",
                [self.runs[1], self.runs[0], self.runs[2]],
            )

    def test_runs_must_share_one_byte_identical_executable(self):
        other_binary = self.binary.with_name("fixture-bin-other")
        other_binary.write_bytes(b"different canonical executable")
        other_binary.chmod(0o755)
        other_sha256 = evidence.file_sha256(other_binary)
        self.rewrite_manifest(
            self.runs[1],
            lambda manifest: manifest.update(
                executable=str(other_binary), executable_sha256=other_sha256
            ),
        )
        with self.assertRaisesRegex(evidence.EvidenceError, "one identical exact executable"):
            evidence.import_evidence(
                self.workspace, self.beta_start, self.root / "artifacts", self.runs
            )

    def test_run_mutation_during_copy_is_rejected(self):
        real_copytree = evidence.shutil.copytree
        copy_count = 0

        def mutating_copytree(source, destination):
            nonlocal copy_count
            result = real_copytree(source, destination)
            copy_count += 1
            if copy_count == 1:
                (pathlib.Path(destination) / "report.json").write_text(
                    "mutated during copy\n", encoding="utf-8"
                )
            return result

        with mock.patch.object(evidence.shutil, "copytree", side_effect=mutating_copytree):
            with self.assertRaisesRegex(evidence.EvidenceError, "changed while"):
                evidence.import_evidence(
                    self.workspace,
                    self.beta_start,
                    self.root / "artifacts",
                    self.runs,
                )


if __name__ == "__main__":
    unittest.main()
