#!/usr/bin/env python3
"""Tests for the tracked release-exception attestation."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release_exception_attestation.py")
SPEC = importlib.util.spec_from_file_location("rvoip_release_exception", SCRIPT)
assert SPEC and SPEC.loader
attestation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(attestation)

SNAPSHOT = (
    SCRIPT.parent.parent
    / "crates/sip/rvoip-sip/docs/releases/beta"
    / "20260729T010954Z/exception-r1"
)


class ReleaseExceptionAttestationTests(unittest.TestCase):
    def test_tracked_0_3_2_snapshot_verifies(self) -> None:
        payload = attestation.verify(
            SNAPSHOT / "exception-attestation.json",
            "0.3.2",
        )
        self.assertEqual(
            payload["release"]["disposition"],
            "APPROVED-WITH-EXCEPTION",
        )
        self.assertEqual(payload["facts"]["gates"]["passed"], 106)
        self.assertEqual(payload["facts"]["gates"]["failed"], 2)
        self.assertEqual(payload["facts"]["gates"]["skipped"], 0)

    def test_tampered_evidence_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            copied = Path(directory) / "snapshot"
            shutil.copytree(SNAPSHOT, copied)
            evidence = copied / "evidence/high-density-caller.json"
            evidence.write_text(evidence.read_text().replace("17871", "17872"))
            with self.assertRaisesRegex(
                attestation.ExceptionAttestationError,
                "hash mismatch",
            ):
                attestation.verify(
                    copied / "exception-attestation.json",
                    "0.3.2",
                )

    def test_release_version_mismatch_fails_closed(self) -> None:
        with self.assertRaisesRegex(
            attestation.ExceptionAttestationError,
            "release version mismatch",
        ):
            attestation.verify(
                SNAPSHOT / "exception-attestation.json",
                "0.3.1",
            )


if __name__ == "__main__":
    unittest.main()
