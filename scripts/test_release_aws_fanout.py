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

    def cutoff_manifest(self) -> dict[str, object]:
        return fanout.prepare_manifest(
            matrix={
                "include": [
                    self.matrix_entry("bounded"),
                    self.matrix_entry(
                        "long-soak",
                        resource="gcp-performance-soak-long",
                        machine="n2-standard-8",
                        gates="perf.long-soak",
                    ),
                    self.matrix_entry(
                        "pbx-interop",
                        resource="gcp-interop",
                        machine="n2-standard-4",
                        gates="interop.pbx",
                    ),
                ]
            },
            candidate=self.candidate,
            environment_id="release-environment",
            run_id="123456789",
            run_attempt="2",
        )

    @staticmethod
    def write_result_only(
        root: Path,
        manifest: dict[str, object],
        shard: str,
        *,
        status: str,
        candidate: str | None = None,
    ) -> None:
        worker = next(item for item in manifest["workers"] if item["id"] == shard)
        directory = root / shard
        directory.mkdir(parents=True, exist_ok=True)
        fanout.write_json(
            directory / "result.json",
            {
                "schema": fanout.RESULT_SCHEMA,
                "candidate_sha": candidate or manifest["candidate_sha"],
                "github_run_id": (
                    f"{manifest['github_run_id']}-{manifest['github_run_attempt']}"
                ),
                "shard_id": shard,
                "gates": sorted(worker["gates"]),
                "exit_code": 0 if status == "PASS" else 1,
                "status": status,
                "evidence_archive_sha256": "0" * 64,
                "publishing_attempted": False,
            },
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

    def test_early_failure_cutoff_waits_for_every_bounded_worker(self) -> None:
        manifest = self.cutoff_manifest()
        states = {worker["name"]: "RUNNING" for worker in manifest["workers"]}
        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertEqual(decision["early_expected"], 1)
            self.assertEqual(decision["early_settled"], 0)
            self.assertFalse(decision["should_stop"])
            self.assertEqual(len(decision["deferred_running"]), 2)

            self.write_result_only(
                downloads, manifest, "long-soak", status="FAIL"
            )
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertEqual(decision["failed_shards"], ["long-soak"])
            self.assertFalse(decision["should_stop"])

    def test_early_failure_cutoff_stops_only_deferred_workers_after_failure(self) -> None:
        manifest = self.cutoff_manifest()
        states = {worker["name"]: "RUNNING" for worker in manifest["workers"]}
        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            self.write_result_only(downloads, manifest, "bounded", status="FAIL")
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertTrue(decision["should_stop"])
            self.assertEqual(decision["early_settled"], 1)
            self.assertEqual(decision["failed_shards"], ["bounded"])
            deferred_names = {
                worker["name"]
                for worker in manifest["workers"]
                if worker["resource_class"] in fanout.DEFERRED_RESOURCE_CLASSES
            }
            self.assertEqual(set(decision["deferred_running"]), deferred_names)

    def test_early_failure_cutoff_never_stops_a_clean_candidate(self) -> None:
        manifest = self.cutoff_manifest()
        states = {worker["name"]: "RUNNING" for worker in manifest["workers"]}
        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            self.write_result_only(downloads, manifest, "bounded", status="PASS")
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertEqual(decision["early_settled"], 1)
            self.assertEqual(decision["failed_shards"], [])
            self.assertFalse(decision["should_stop"])

    def test_partial_result_preserves_completed_gates_but_never_qualifies(self) -> None:
        manifest = self.cutoff_manifest()
        worker = next(item for item in manifest["workers"] if item["id"] == "long-soak")
        result = {
            "schema": fanout.RESULT_SCHEMA,
            "candidate_sha": manifest["candidate_sha"],
            "github_run_id": (
                f"{manifest['github_run_id']}-{manifest['github_run_attempt']}"
            ),
            "shard_id": "long-soak",
            "gates": sorted(worker["gates"]),
            "completed_gates": ["perf.long-soak"],
            "exit_code": 143,
            "status": "PARTIAL",
            "evidence_archive_sha256": "0" * 64,
            "publishing_attempted": False,
        }
        self.assertFalse(
            fanout.validate_result(worker=worker, result=result, manifest=manifest)
        )

        result["completed_gates"] = ["not.in.this.shard"]
        with self.assertRaisesRegex(fanout.FanoutError, "invalid completed_gates"):
            fanout.validate_result(worker=worker, result=result, manifest=manifest)

    def test_early_failure_cutoff_fails_closed_on_invalid_or_missing_evidence(self) -> None:
        manifest = self.cutoff_manifest()
        states = {worker["name"]: "RUNNING" for worker in manifest["workers"]}
        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            self.write_result_only(
                downloads,
                manifest,
                "bounded",
                status="PASS",
                candidate="d" * 40,
            )
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertTrue(decision["should_stop"])
            self.assertIn("bounded", decision["invalid_results"])

        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            bounded = next(
                worker for worker in manifest["workers"] if worker["id"] == "bounded"
            )
            states[bounded["name"]] = "TERMINATED"
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertTrue(decision["should_stop"])
            self.assertEqual(decision["failed_shards"], ["bounded"])

    def test_instance_state_csv_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "states.csv"
            path.write_text("worker-1,RUNNING\nworker-2,TERMINATED\n")
            self.assertEqual(
                fanout.load_instance_states(path),
                {"worker-1": "RUNNING", "worker-2": "TERMINATED"},
            )
            path.write_text("worker-1,RUNNING,extra\n")
            with self.assertRaisesRegex(fanout.FanoutError, "invalid GCP"):
                fanout.load_instance_states(path)

    @staticmethod
    def write_archive(
        path: Path,
        member_name: str,
        payload: bytes,
        *,
        sidecars: dict[str, bytes] | None = None,
    ) -> str:
        with tarfile.open(path, "w:gz") as bundle:
            member = tarfile.TarInfo(member_name)
            member.size = len(payload)
            bundle.addfile(member, io.BytesIO(payload))
            for sidecar_name, sidecar_payload in (sidecars or {}).items():
                sidecar = tarfile.TarInfo(sidecar_name)
                sidecar.size = len(sidecar_payload)
                bundle.addfile(sidecar, io.BytesIO(sidecar_payload))
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
            archive_sha = self.write_archive(
                archive,
                member,
                b'{"status":"PASS"}\n',
                sidecars={
                    "release-shard/_sccache-stats.txt": f"{shard}\n".encode()
                },
            )
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
                self.assertEqual(
                    (
                        output
                        / fanout.WORKER_EVIDENCE_DIR
                        / worker["id"]
                        / "_sccache-stats.txt"
                    ).read_text(),
                    f"{worker['id']}\n",
                )
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

    def test_verify_keeps_gate_evidence_duplicates_fail_closed(self) -> None:
        manifest = self.manifest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest)
            for worker in manifest["workers"]:
                shard = worker["id"]
                archive = downloads / shard / "release-shard.tar.gz"
                archive_sha = self.write_archive(
                    archive,
                    "release-shard/perf.shared/receipt.json",
                    b'{"status":"PASS"}\n',
                )
                result_path = downloads / shard / "result.json"
                result = json.loads(result_path.read_text())
                result["evidence_archive_sha256"] = archive_sha
                fanout.write_json(result_path, result)

            receipt = fanout.verify_fanout(
                manifest=manifest,
                downloads=downloads,
                output=root / "release-shard",
            )
            self.assertEqual(receipt["status"], "FAIL")
            self.assertEqual(receipt["trusted_shards"], [manifest["workers"][0]["id"]])
            self.assertTrue(
                any("duplicate evidence path" in error for error in receipt["errors"])
            )

    def test_verify_rejects_worker_archives_using_controller_namespaces(self) -> None:
        manifest = self.manifest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "downloads"
            downloads.mkdir()
            self.populate_downloads(downloads, manifest)
            worker = manifest["workers"][0]
            archive = downloads / worker["id"] / "release-shard.tar.gz"
            archive_sha = self.write_archive(
                archive,
                "release-shard/_gcp-controller/injected.json",
                b"{}\n",
            )
            result_path = downloads / worker["id"] / "result.json"
            result = json.loads(result_path.read_text())
            result["evidence_archive_sha256"] = archive_sha
            fanout.write_json(result_path, result)

            receipt = fanout.verify_fanout(
                manifest=manifest,
                downloads=downloads,
                output=root / "release-shard",
            )
            self.assertEqual(receipt["status"], "FAIL")
            self.assertTrue(
                any("reserved controller path" in error for error in receipt["errors"])
            )

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
