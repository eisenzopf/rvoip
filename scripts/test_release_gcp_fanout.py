from __future__ import annotations

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release") / "gcp_fanout.py"
SPEC = importlib.util.spec_from_file_location("release_gcp_fanout", SCRIPT)
assert SPEC and SPEC.loader
fanout = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fanout)


class GcpReleaseFanoutTests(unittest.TestCase):
    candidate = "c" * 40

    @staticmethod
    def matrix_entry(
        shard: str,
        *,
        resource: str = "gcp-performance",
        machine: str = "n2-standard-8",
        gates: str = "perf.one,perf.two",
        disk_size_gb: int = 200,
    ) -> dict[str, object]:
        return {
            "id": shard,
            "resource_class": resource,
            "machine_type": machine,
            "disk_type": "pd-standard",
            "disk_size_gb": disk_size_gb,
            "gates_csv": gates,
        }

    def manifest(self) -> dict[str, object]:
        return fanout.prepare_manifest(
            matrix={
                "include": [
                    self.matrix_entry("gcp-performance-1"),
                    self.matrix_entry(
                        "gcp-performance-soak-1",
                        resource="gcp-performance-soak",
                        machine="n2-standard-4",
                        gates="perf.soak",
                    ),
                ]
            },
            candidate=self.candidate,
            environment_id="release-environment",
            run_id="123456789",
            run_attempt="2",
        )

    def test_prepare_is_deterministic_and_capacity_aware(self) -> None:
        manifest = self.manifest()
        self.assertEqual(manifest["worker_count"], 2)
        self.assertEqual(manifest["required_vcpus"], 12)
        workers = manifest["workers"]
        self.assertEqual(
            [worker["id"] for worker in workers],
            ["gcp-performance-1", "gcp-performance-soak-1"],
        )
        self.assertEqual(
            workers[0]["name"], "rvoip-rel-123456789-2-gcp-performance-1"
        )
        self.assertEqual(workers[0]["prefix"], "release/123456789-2/gcp-performance-1")
        self.assertEqual(workers[0]["gates_b64"], "cGVyZi5vbmUscGVyZi50d28=")
        fanout.validate_manifest(manifest)

    def test_prepare_rejects_duplicate_shards_and_machine_downgrades(self) -> None:
        duplicate = self.matrix_entry("gcp-performance-1")
        with self.assertRaisesRegex(fanout.FanoutError, "duplicate GCP shard"):
            fanout.prepare_manifest(
                matrix={"include": [duplicate, duplicate]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )
        with self.assertRaisesRegex(fanout.FanoutError, "must use n2-standard-8"):
            fanout.prepare_manifest(
                matrix={
                    "include": [
                        self.matrix_entry(
                            "gcp-performance-1", machine="n2-standard-4"
                        )
                    ]
                },
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )

        proxy = self.matrix_entry(
            "gcp-proxy-interop-1",
            resource="gcp-proxy-interop",
            machine="n2-standard-2",
            gates="interop.remote-proxies.kamailio.rvoip-first.udp",
            disk_size_gb=100,
        )
        manifest = fanout.prepare_manifest(
            matrix={"include": [proxy]},
            candidate=self.candidate,
            environment_id="release-environment",
            run_id="1",
            run_attempt="1",
        )
        self.assertEqual(manifest["required_vcpus"], 2)
        self.assertEqual(manifest["workers"][0]["disk_size_gb"], 100)
        proxy["disk_size_gb"] = 200
        with self.assertRaisesRegex(fanout.FanoutError, "must use a 100 GB boot disk"):
            fanout.prepare_manifest(
                matrix={"include": [proxy]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )

        long_soak = self.matrix_entry(
            "gcp-performance-soak-long-1",
            resource="gcp-performance-soak-long",
            machine="n2-standard-8",
            gates="perf.soak-candidate",
        )
        manifest = fanout.prepare_manifest(
            matrix={"include": [long_soak]},
            candidate=self.candidate,
            environment_id="release-environment",
            run_id="1",
            run_attempt="1",
        )
        self.assertEqual(manifest["required_vcpus"], 8)
        long_soak["machine_type"] = "n2-standard-4"
        with self.assertRaisesRegex(fanout.FanoutError, "must use n2-standard-8"):
            fanout.prepare_manifest(
                matrix={"include": [long_soak]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )

    @staticmethod
    def write_archive(path: Path, member_name: str, payload: bytes) -> str:
        with tarfile.open(path, "w:gz") as bundle:
            member = tarfile.TarInfo(member_name)
            member.size = len(payload)
            bundle.addfile(member, io.BytesIO(payload))
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def populate_downloads(
        self, root: Path, manifest: dict[str, object], *, unsafe: bool = False
    ) -> None:
        expected_run = (
            f"{manifest['github_run_id']}-{manifest['github_run_attempt']}"
        )
        for index, worker in enumerate(manifest["workers"]):
            shard = worker["id"]
            directory = root / shard
            directory.mkdir(parents=True)
            archive = directory / "release-shard.tar.gz"
            member = (
                "release-shard/../escape"
                if unsafe and index == 0
                else f"release-shard/{shard}/receipt.json"
            )
            archive_sha = self.write_archive(archive, member, b'{"status":"PASS"}\n')
            result = {
                "schema": fanout.RESULT_SCHEMA,
                "candidate_sha": manifest["candidate_sha"],
                "github_run_id": expected_run,
                "shard_id": shard,
                "gates": sorted(worker["gates"]),
                "exit_code": 0,
                "status": "PASS",
                "evidence_archive_sha256": archive_sha,
                "publishing_attempted": False,
            }
            fanout.write_json(directory / "result.json", result)
            (directory / "qualification.log").write_text("passed\n")

    def test_verify_merges_every_shard_after_binding_all_evidence(self) -> None:
        manifest = self.manifest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest)
            output = root / "release-shard"
            receipt = fanout.verify_fanout(
                manifest=manifest, downloads=downloads, output=output
            )
            self.assertEqual(receipt["status"], "PASS")
            self.assertEqual(receipt["worker_count"], 2)
            for worker in manifest["workers"]:
                self.assertTrue((output / worker["id"] / "receipt.json").is_file())
                self.assertTrue(
                    (
                        output
                        / "_gcp-controller"
                        / worker["id"]
                        / "result.json"
                    ).is_file()
                )
            fanout_receipt = json.loads(
                (output / "_gcp-controller" / "fanout-receipt.json").read_text()
            )
            self.assertFalse(fanout_receipt["publishing_attempted"])

    def test_verify_rejects_tampering_and_archive_traversal(self) -> None:
        manifest = self.manifest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest)
            worker = manifest["workers"][0]
            archive = downloads / worker["id"] / "release-shard.tar.gz"
            archive.write_bytes(archive.read_bytes() + b"tampered")
            receipt = fanout.verify_fanout(
                manifest=manifest,
                downloads=downloads,
                output=root / "release-shard",
            )
            self.assertEqual(receipt["status"], "FAIL")
            self.assertIn(worker["id"], receipt["failed_shards"])
            self.assertTrue(any("digest mismatch" in error for error in receipt["errors"]))

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest, unsafe=True)
            receipt = fanout.verify_fanout(
                manifest=manifest,
                downloads=downloads,
                output=root / "release-shard",
            )
            self.assertEqual(receipt["status"], "FAIL")
            self.assertTrue(
                any("unsafe evidence archive" in error for error in receipt["errors"])
            )

    def test_failed_shard_preserves_all_bound_evidence_for_selective_retry(self) -> None:
        manifest = self.manifest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest)
            failed = manifest["workers"][0]
            result_path = downloads / failed["id"] / "result.json"
            result = json.loads(result_path.read_text())
            result["status"] = "FAIL"
            result["exit_code"] = 1
            fanout.write_json(result_path, result)

            output = root / "release-shard"
            receipt = fanout.verify_fanout(
                manifest=manifest, downloads=downloads, output=output
            )
            self.assertEqual(receipt["status"], "FAIL")
            self.assertEqual(receipt["failed_shards"], [failed["id"]])
            self.assertEqual(len(receipt["trusted_shards"]), 2)
            for worker in manifest["workers"]:
                self.assertTrue((output / worker["id"] / "receipt.json").is_file())


if __name__ == "__main__":
    unittest.main()
