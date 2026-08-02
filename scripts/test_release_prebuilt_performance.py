#!/usr/bin/env python3
"""Contract tests for exact-candidate performance prebuilds."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release") / "prebuilt_performance.py"
SPEC = importlib.util.spec_from_file_location("prebuilt_performance", SCRIPT)
assert SPEC and SPEC.loader
prebuilt = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prebuilt)
ROOT = Path(__file__).resolve().parents[1]


class PrebuiltPerformanceTests(unittest.TestCase):
    def test_exact_burst_gate_selects_both_binaries_once(self) -> None:
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        groups, definitions = prebuilt.selected_builds(
            ROOT,
            catalog,
            ["perf.media-burst.access-edge-microburst"],
        )
        self.assertEqual(
            groups,
            {
                ("perf-tests",): {
                    "perf_burst_caller",
                    "perf_burst_receiver",
                }
            },
        )
        self.assertEqual(
            definitions["perf.media-burst.access-edge-microburst"]["resolved_targets"],
            ["perf_burst_caller", "perf_burst_receiver"],
        )

    def test_direct_cargo_gate_preserves_environment_and_libtest_arguments(
        self,
    ) -> None:
        definition = prebuilt.cargo_test_definition(
            [
                "env",
                "RVOIP_PERF_ARCHIVE_DIR={artifact_dir}",
                "cargo",
                "test",
                "--locked",
                "-p",
                "rvoip-sip",
                "--release",
                "--features",
                "dev-insecure-tls,perf-tests",
                "--test",
                "perf_soak_30min",
                "perf_mass_teardown_stress",
                "--",
                "--exact",
                "--ignored",
                "--nocapture",
            ]
        )
        assert definition is not None
        self.assertEqual(
            definition["environment"],
            {"RVOIP_PERF_ARCHIVE_DIR": "{artifact_dir}"},
        )
        self.assertEqual(definition["features"], ("dev-insecure-tls", "perf-tests"))
        self.assertEqual(definition["targets"], ["perf_soak_30min"])
        self.assertEqual(
            definition["runner_args"],
            ["perf_mass_teardown_stress", "--exact", "--ignored", "--nocapture"],
        )

    def test_wildcard_targets_expand_from_real_workspace(self) -> None:
        matches = prebuilt.expand_targets(
            ["resilien*"], prebuilt.available_test_targets(ROOT)
        )
        self.assertGreaterEqual(len(matches), 1)
        self.assertTrue(all(target.startswith("resilien") for target in matches))

    def test_result_is_bound_to_candidate_environment_and_gate_set(self) -> None:
        candidate = "a" * 40
        payload = {
            "schema": prebuilt.RESULT_SCHEMA,
            "candidate_sha": candidate,
            "environment_id": "environment-v1",
            "selected_gate_ids": ["perf.one", "perf.two"],
            "status": "PASS",
            "exit_code": 0,
            "bundle_uri": "gs://bucket/release/1/prebuild/performance-prebuilt.tar.gz",
            "bundle_sha256": "b" * 64,
            "manifest_uri": "gs://bucket/release/1/prebuild/performance-manifest.json",
            "manifest_sha256": "d" * 64,
            "publishing_attempted": False,
        }
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(json.dumps(payload))
            verified = prebuilt.validate_result(
                result,
                candidate=candidate,
                environment_id="environment-v1",
                gate_ids=["perf.two", "perf.one"],
            )
            self.assertEqual(verified["bundle_sha256"], "b" * 64)
            payload["candidate_sha"] = "c" * 40
            result.write_text(json.dumps(payload))
            with self.assertRaisesRegex(prebuilt.PrebuiltError, "candidate_sha"):
                prebuilt.validate_result(
                    result,
                    candidate=candidate,
                    environment_id="environment-v1",
                    gate_ids=["perf.one", "perf.two"],
                )

    def test_bundle_rejects_path_traversal_before_extraction(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "bundle.tar.gz"
            with tarfile.open(archive, "w:gz") as bundle:
                member = tarfile.TarInfo("performance-prebuilt/../escape")
                member.size = 1
                bundle.addfile(member, io.BytesIO(b"x"))
            self.assertEqual(
                prebuilt.sha256_file(archive),
                hashlib.sha256(archive.read_bytes()).hexdigest(),
            )
            with self.assertRaisesRegex(prebuilt.PrebuiltError, "unsafe"):
                prebuilt.safe_bundle_members(archive)

    def test_non_binary_preflight_does_not_request_a_bundle(self) -> None:
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        gate = next(
            item
            for item in catalog["gates"]
            if item["id"] == "preflight.performance-01"
        )
        self.assertIsNone(prebuilt.gate_definition(gate))

    def test_every_runtime_performance_command_has_prebuild_support(self) -> None:
        catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
        unsupported = []
        for gate in catalog["gates"]:
            if (
                str(gate.get("resource_class", "")).startswith("gcp-performance")
                and gate.get("executor") == "argv"
                and not gate["id"].startswith("preflight.performance")
                and prebuilt.gate_definition(gate) is None
            ):
                unsupported.append(gate["id"])
        self.assertEqual(unsupported, [])


if __name__ == "__main__":
    unittest.main()
