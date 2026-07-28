#!/usr/bin/env python3
"""Tests for immutable performance-regression baseline packaging."""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("perf_regression_baseline.py")
TRACKED_BASELINE = SCRIPT.parent.parent / "perf-baselines/20260706T181609Z"


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


class PerfRegressionBaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.payload = b'{"scenario":"fixture","results":{"achieved_cps":1}}\n'
        (self.source / "fixture.json").write_bytes(self.payload)
        self.manifest = self.root / "manifest.json"
        self.manifest.write_text(
            json.dumps(
                {
                    "schema": "rvoip-perf-regression-baseline-v1",
                    "baseline_id": "20260706T181609Z",
                    "qualification": {
                        "release_evidence": False,
                        "permitted_use": "reviewed regression threshold only",
                    },
                    "source": {"git_revision": "fixture", "git_status": "dirty"},
                    "comparison_paths": ["fixture.json"],
                    "files": [
                        {
                            "path": "fixture.json",
                            "bytes": len(self.payload),
                            "sha256": digest(self.payload),
                        }
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_helper(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), *arguments],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_verify_and_package_preserve_exact_hashes(self) -> None:
        verified = self.run_helper(
            "verify",
            "--manifest",
            str(self.manifest),
            "--result-root",
            str(self.source),
        )
        self.assertEqual(verified.returncode, 0, verified.stderr)

        artifacts = self.root / "artifacts"
        packaged = self.run_helper(
            "package",
            "--manifest",
            str(self.manifest),
            "--source-root",
            str(self.source),
            "--artifact-dir",
            str(artifacts),
        )
        self.assertEqual(packaged.returncode, 0, packaged.stderr)
        package = artifacts / "perf-regression-baseline"
        self.assertEqual(
            (package / "perf-results/fixture.json").read_bytes(), self.payload
        )
        self.assertEqual(
            json.loads((package / "manifest.json").read_text()),
            json.loads(self.manifest.read_text()),
        )

    def test_tracked_reviewed_baseline_verifies(self) -> None:
        result = self.run_helper(
            "verify",
            "--manifest",
            str(TRACKED_BASELINE / "manifest.json"),
            "--result-root",
            str(TRACKED_BASELINE),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("20260706T181609Z (6 files)", result.stdout)

    def test_changed_file_fails_closed(self) -> None:
        (self.source / "fixture.json").write_bytes(self.payload + b" ")
        result = self.run_helper(
            "verify",
            "--manifest",
            str(self.manifest),
            "--result-root",
            str(self.source),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("byte count changed", result.stderr)

    def test_unlisted_file_fails_closed(self) -> None:
        (self.source / "unlisted.json").write_text("{}\n", encoding="utf-8")
        result = self.run_helper(
            "verify",
            "--manifest",
            str(self.manifest),
            "--result-root",
            str(self.source),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unlisted unlisted.json", result.stderr)

    def test_traversal_path_is_rejected(self) -> None:
        value = json.loads(self.manifest.read_text())
        value["files"][0]["path"] = "../fixture.json"
        self.manifest.write_text(json.dumps(value), encoding="utf-8")
        result = self.run_helper(
            "verify",
            "--manifest",
            str(self.manifest),
            "--result-root",
            str(self.source),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("not a safe relative path", result.stderr)

    def test_duplicate_comparison_path_is_rejected(self) -> None:
        value = json.loads(self.manifest.read_text())
        value["comparison_paths"].append("fixture.json")
        self.manifest.write_text(json.dumps(value), encoding="utf-8")
        result = self.run_helper(
            "verify",
            "--manifest",
            str(self.manifest),
            "--result-root",
            str(self.source),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("comparison paths must be unique", result.stderr)


if __name__ == "__main__":
    unittest.main()
