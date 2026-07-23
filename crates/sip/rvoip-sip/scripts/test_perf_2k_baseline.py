#!/usr/bin/env python3
"""Focused checks for the canonical 2,000-CPS reviewed baseline input."""

import hashlib
import importlib.util
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
WORKSPACE_ROOT = SCRIPT_DIR.parents[3]
PROFILE_SCRIPT = SCRIPT_DIR / "perf_call_setup_2k_profile.sh"
PROFILE_SOURCE = PROFILE_SCRIPT.read_text(encoding="utf-8")


def shell_constant(name):
    match = re.search(rf'^{re.escape(name)}="([^"]+)"$', PROFILE_SOURCE, re.MULTILINE)
    if match is None:
        raise AssertionError(f"missing shell constant {name}")
    return match.group(1)


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


canonical = load_module(
    "test_perf_2k_baseline_canonical", SCRIPT_DIR / "canonical_2k_evidence.py"
)
attestation = load_module(
    "test_perf_2k_baseline_attestation", SCRIPT_DIR / "beta_attestation.py"
)


class ReviewedBaselineTests(unittest.TestCase):
    def setUp(self):
        self.baseline_id = shell_constant("REVIEWED_BASELINE_ID")
        self.scenario = shell_constant("REVIEWED_BASELINE_SCENARIO")
        self.expected_sha256 = shell_constant(
            "REVIEWED_BASELINE_CANONICAL_SHA256"
        )
        self.relative_path = f"{self.scenario}/2000.json"
        self.baseline_root = (
            SCRIPT_DIR.parent / "perf-baselines" / self.baseline_id
        )
        self.baseline = self.baseline_root / self.relative_path

    def test_default_baseline_is_present_under_nonignored_source_path(self):
        self.assertIn(
            'TRACKED_REVIEWED_BASELINE="${CRATE_DIR}/perf-baselines/${REVIEWED_BASELINE_ID}"',
            PROFILE_SOURCE,
        )
        self.assertNotIn(
            'REVIEWED_BASELINE="${RVOIP_PERF_REVIEWED_BASELINE:-${CRATE_DIR}/beta-report',
            PROFILE_SOURCE,
        )
        self.assertTrue(self.baseline.is_file())
        ignored = subprocess.run(
            ["git", "check-ignore", "--quiet", str(self.baseline)],
            cwd=WORKSPACE_ROOT,
            check=False,
        )
        self.assertEqual(ignored.returncode, 1, "reviewed baseline must not be ignored")

    def test_reviewed_baseline_hash_and_all_validators_agree(self):
        actual = hashlib.sha256(self.baseline.read_bytes()).hexdigest()
        self.assertEqual(actual, self.expected_sha256)
        self.assertEqual(canonical.REVIEWED_BASELINE_ID, self.baseline_id)
        self.assertEqual(canonical.REVIEWED_BASELINE_RELATIVE_PATH, self.relative_path)
        self.assertEqual(canonical.REVIEWED_BASELINE_SHA256, actual)
        self.assertEqual(attestation.CANONICAL_2K_BASELINE_ID, self.baseline_id)
        self.assertEqual(
            attestation.CANONICAL_2K_BASELINE_RELATIVE_PATH, self.relative_path
        )
        self.assertEqual(attestation.CANONICAL_2K_BASELINE_SHA256, actual)

    def test_clean_run_snapshots_complete_verified_baseline_inventory(self):
        self.assertIn(
            'cp -R "${REVIEWED_BASELINE}/." "${REVIEWED_BASELINE_SNAPSHOT}/"',
            PROFILE_SOURCE,
        )
        self.assertIn(
            '"${SCRIPT_DIR}/perf_regression_baseline.py" verify',
            PROFILE_SOURCE,
        )
        self.assertIn(
            '--manifest "${REVIEWED_BASELINE_SNAPSHOT}/manifest.json"',
            PROFILE_SOURCE,
        )
        self.assertIn(
            '--result-root "${REVIEWED_BASELINE_SNAPSHOT}"',
            PROFILE_SOURCE,
        )
        self.assertNotIn(
            'cp "${REVIEWED_BASELINE_REPORT}" "${snapshot_report}"',
            PROFILE_SOURCE,
        )

    def test_override_requires_declared_hash_before_any_cargo_build(self):
        with tempfile.TemporaryDirectory() as temp:
            override_root = pathlib.Path(temp)
            override = override_root / self.relative_path
            override.parent.mkdir(parents=True)
            shutil.copyfile(self.baseline, override)
            environment = os.environ.copy()
            for name in list(environment):
                if name in {
                    "RUSTFLAGS",
                    "CARGO_ENCODED_RUSTFLAGS",
                    "CARGO_BUILD_RUSTFLAGS",
                    "CARGO_INCREMENTAL",
                } or name.startswith(("CARGO_PROFILE_RELEASE_", "MIMALLOC_")) or (
                    name.startswith("CARGO_TARGET_") and name.endswith("_RUSTFLAGS")
                ):
                    environment.pop(name, None)
            environment["RVOIP_PERF_REVIEWED_BASELINE"] = str(override_root)
            environment.pop("RVOIP_PERF_REVIEWED_BASELINE_SHA256", None)
            result = subprocess.run(
                [str(PROFILE_SCRIPT), "clean"],
                cwd=WORKSPACE_ROOT,
                env=environment,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn(
                "requires RVOIP_PERF_REVIEWED_BASELINE_SHA256", result.stderr
            )

            environment["RVOIP_PERF_REVIEWED_BASELINE_SHA256"] = "0" * 64
            result = subprocess.run(
                [str(PROFILE_SCRIPT), "clean"],
                cwd=WORKSPACE_ROOT,
                env=environment,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("does not match its declared SHA-256", result.stderr)


if __name__ == "__main__":
    unittest.main()
