#!/usr/bin/env python3
"""Fail-closed tests for the 0.3.4 carry-forward release attestation."""

from __future__ import annotations

import importlib.util
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("release_carry_forward_attestation.py")
SPEC = importlib.util.spec_from_file_location("rvoip_carry_forward", SCRIPT)
assert SPEC and SPEC.loader
carry = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(carry)


SOURCE = {
    "git_commit": "a" * 40,
    "git_rev": "a" * 8,
    "git_tree": "b" * 40,
    "git_dirty": False,
    "source_fingerprint_sha256": "c" * 64,
}
DELTA = {
    "base_version": "0.3.3",
    "base_tag": "v0.3.3",
    "base_commit": "d" * 40,
    "changed_path_count": 1,
    "changed_paths": ["scripts/release.py"],
}
INHERITED = {
    "version": "0.3.2",
    "disposition": "APPROVED-WITH-EXCEPTION",
    "strict_automated_status": "NON-RC",
    "attestation_path": carry.INHERITED_RELATIVE_PATH,
    "attestation_sha256": carry.INHERITED_SHA256,
}


class FakeAcceptance:
    @staticmethod
    def evaluate(report: dict, scenario: str, path: Path) -> dict:
        del report, scenario, path
        return {"status": "PASS", "checks": [{"passed": True}]}


class FakeCanonical:
    ACCEPTANCE = FakeAcceptance()
    CANONICAL_SCENARIO = "perf_call_setup_cps_pbx-media-server"

    @staticmethod
    def tree_sha256(root: Path) -> str:
        digest = hashlib.sha256(b"rvoip-canonical-evidence-tree-v1\0")
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(root).as_posix().encode()
            for value in (relative, path.read_bytes()):
                digest.update(len(value).to_bytes(8, "little"))
                digest.update(value)
        return digest.hexdigest()


def report() -> dict:
    convergence = {
        "converged": True,
        "retained_total": 0,
        "missing_count": 0,
    }
    return {
        "scenario": "perf_call_setup_cps_pbx-media-server",
        "results": {
            "calls_offered": 65000,
            "calls_succeeded": 65000,
            "achieved_cps": 1857.1,
            "asr": 1.0,
            "ner": 1.0,
            "errors": {
                "answer_failed": 0,
                "bye_failed": 0,
                "invite_send_failed": 0,
                "timeout": 0,
            },
        },
        "latency_ns": {"setup_latency": {"p99": 1_200_000}},
        "resources": {
            "peak_rss_mb": 2155.7,
            "rss_active_growth_mb_per_min": 1558.6,
        },
        "diagnostics": {
            "cleanup_convergence_at_settle": convergence,
            "cleanup_convergence": convergence,
        },
    }


def canonical_payload(value: dict, evidence_tree_sha256: str) -> dict:
    facts = carry.canonical_facts(value)
    facts.update(
        {
            "captured_at_utc": "2026-07-30T00:00:00Z",
            "source_fingerprint_sha256": SOURCE["source_fingerprint_sha256"],
            "executable_sha256": "e" * 64,
            "evidence_tree_sha256": evidence_tree_sha256,
        }
    )
    return facts


def write_fixture(root: Path) -> Path:
    evidence = root / "evidence/canonical-2k"
    evidence.mkdir(parents=True)
    current_report = report()
    (evidence / "report.json").write_text(json.dumps(current_report))
    (evidence / "acceptance.json").write_text(
        json.dumps({"schema": "rvoip-sip-2k-acceptance-v3", "status": "PASS"})
    )
    (evidence / "build-environment.json").write_text("{}")
    (evidence / "source-at-build.json").write_text("{}")
    (evidence / "source-at-finalize.json").write_text(
        json.dumps(
            {
                "git_commit": SOURCE["git_commit"],
                "git_dirty": False,
                "source_fingerprint_sha256": SOURCE[
                    "source_fingerprint_sha256"
                ],
            }
        )
    )
    (evidence / "manifest.json").write_text(
        json.dumps(
            {
                "overall_status": "PASS",
                "acceptance_status": "PASS",
                "perf_audit_status": "PASS",
                "executable_sha256": "e" * 64,
                "source_at_finalize": {
                    "source_fingerprint_sha256": SOURCE[
                        "source_fingerprint_sha256"
                    ]
                },
            }
        )
    )
    (evidence / "perf-audit.md").write_text("status: OK\n")
    inherited = root / "inherited"
    inherited.mkdir()
    shutil.copy2(
        carry.repository_root() / carry.INHERITED_RELATIVE_PATH,
        inherited / "exception-attestation.json",
    )
    evidence_tree_sha256 = FakeCanonical.tree_sha256(evidence)
    payload = {
        "schema": carry.SCHEMA,
        "created_at_utc": "2026-07-30T00:00:00+00:00",
        "release": {
            "version": "0.3.4",
            "git_commit": SOURCE["git_commit"],
            "git_tree": SOURCE["git_tree"],
            "source_fingerprint_sha256": SOURCE["source_fingerprint_sha256"],
            "disposition": "OWNER-APPROVED-CARRY-FORWARD",
            "beta_suite": "NOT-RERUN",
            "current_workspace_verification": "REQUIRED-PASS",
        },
        "release_delta": DELTA,
        "inherited_beta_background": INHERITED,
        "current_evidence": {
            "canonical_2k": canonical_payload(
                current_report, evidence_tree_sha256
            ),
            "scope": "one clean canonical real-media run plus current workspace verification",
            "full_beta_suite": "NOT-RERUN",
            "interoperability_matrix": "NOT-RERUN",
            "long_soaks": "NOT-RERUN",
        },
        "approval": {
            "actor": "project owner/operator",
            "basis": "Explicitly approved carry-forward release.",
            "method": "explicit project-owner/operator instruction",
            "cryptographically_signed": False,
        },
        "artifacts": carry.artifact_manifest(root),
    }
    attestation = root / "carry-forward-attestation.json"
    carry.write_json(attestation, payload)
    (root / "carry-forward-attestation.json.sha256").write_text(
        f"{carry.sha256(attestation)}  carry-forward-attestation.json\n"
    )
    return attestation


def rewrite_checksum(attestation: Path) -> None:
    attestation.with_name("carry-forward-attestation.json.sha256").write_text(
        f"{carry.sha256(attestation)}  carry-forward-attestation.json\n"
    )


class CarryForwardTests(unittest.TestCase):
    def verify(self, attestation: Path) -> None:
        with (
            mock.patch.object(carry, "current_source", return_value=SOURCE),
            mock.patch.object(carry, "release_delta", return_value=DELTA),
            mock.patch.object(
                carry, "validate_inherited_exception", return_value=INHERITED
            ),
            mock.patch.object(carry, "canonical_helper", return_value=FakeCanonical()),
        ):
            carry.verify(attestation, "0.3.4")

    def test_valid_attestation_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.verify(write_fixture(Path(directory)))

    def test_wrong_release_version_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            attestation = write_fixture(Path(directory))
            with self.assertRaises(carry.CarryForwardError):
                carry.verify(attestation, "0.3.5")

    def test_stale_commit_and_changed_fingerprint_fail(self) -> None:
        for field, value in (
            ("git_commit", "0" * 40),
            ("source_fingerprint_sha256", "1" * 64),
        ):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                attestation = write_fixture(Path(directory))
                payload = json.loads(attestation.read_text())
                payload["release"][field] = value
                carry.write_json(attestation, payload)
                rewrite_checksum(attestation)
                with self.assertRaises(carry.CarryForwardError):
                    self.verify(attestation)

    def test_altered_or_missing_performance_evidence_fails(self) -> None:
        for action in ("alter", "remove"):
            with self.subTest(action=action), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                attestation = write_fixture(root)
                report_path = root / "evidence/canonical-2k/report.json"
                if action == "alter":
                    report_path.write_text("{}")
                else:
                    report_path.unlink()
                with self.assertRaises(carry.CarryForwardError):
                    self.verify(attestation)

    def test_unrun_beta_cannot_be_relabelled_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            attestation = write_fixture(Path(directory))
            payload = json.loads(attestation.read_text())
            payload["current_evidence"]["full_beta_suite"] = "PASS"
            carry.write_json(attestation, payload)
            rewrite_checksum(attestation)
            with self.assertRaises(carry.CarryForwardError):
                self.verify(attestation)

    def test_canonical_threshold_and_cleanup_fail_closed(self) -> None:
        for label, mutate in (
            (
                "cps",
                lambda value: value["results"].update({"achieved_cps": 1000}),
            ),
            (
                "error",
                lambda value: value["results"]["errors"].update(
                    {"bye_failed": 1}
                ),
            ),
            (
                "retention",
                lambda value: value["diagnostics"][
                    "cleanup_convergence_at_settle"
                ].update({"retained_total": 1}),
            ),
        ):
            with self.subTest(label=label):
                value = report()
                mutate(value)
                with self.assertRaises(carry.CarryForwardError):
                    carry.canonical_facts(value)


if __name__ == "__main__":
    unittest.main()
