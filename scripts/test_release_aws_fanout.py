from __future__ import annotations

import base64
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release") / "aws_fanout.py"
SPEC = importlib.util.spec_from_file_location("release_aws_fanout", SCRIPT)
assert SPEC and SPEC.loader
fanout = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fanout)


class AwsReleaseFanoutTests(unittest.TestCase):
    candidate = "c" * 40

    @staticmethod
    def matrix_entry(
        shard: str,
        *,
        resource: str = "ec2-performance",
        machine: str = "m5.2xlarge",
        gates: str = "perf.one,perf.two",
        disk_size_gb: int = 200,
    ) -> dict[str, object]:
        return {
            "id": shard,
            "resource_class": resource,
            "machine_type": machine,
            "disk_type": "gp3",
            "disk_size_gb": disk_size_gb,
            "gates_csv": gates,
        }

    def manifest(self) -> dict[str, object]:
        return fanout.prepare_manifest(
            matrix={
                "include": [
                    self.matrix_entry("ec2-performance-1"),
                    self.matrix_entry(
                        "ec2-performance-soak-1",
                        resource="ec2-performance-soak",
                        machine="m5.xlarge",
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
                        resource="ec2-performance-soak-long",
                        machine="m5.2xlarge",
                        gates="perf.long-soak",
                    ),
                    self.matrix_entry(
                        "pbx-interop",
                        resource="ec2-interop",
                        machine="m5.xlarge",
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
            ["ec2-performance-1", "ec2-performance-soak-1"],
        )
        self.assertEqual(
            workers[0]["name"], "rvoip-rel-123456789-2-ec2-performance-1"
        )
        self.assertEqual(workers[0]["prefix"], "release/123456789-2/ec2-performance-1")
        self.assertEqual(workers[0]["gates_b64"], "cGVyZi5vbmUscGVyZi50d28=")
        fanout.validate_manifest(manifest)

    def test_prepare_rejects_duplicate_shards_and_machine_downgrades(self) -> None:
        duplicate = self.matrix_entry("ec2-performance-1")
        with self.assertRaisesRegex(fanout.FanoutError, "255-character tag limit"):
            fanout.prepare_manifest(
                matrix={"include": [self.matrix_entry("a" * 48)]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="9" * 250,
                run_attempt="1",
            )
        with self.assertRaisesRegex(fanout.FanoutError, "duplicate EC2 shard"):
            fanout.prepare_manifest(
                matrix={"include": [duplicate, duplicate]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )
        with self.assertRaisesRegex(fanout.FanoutError, "must use m5.2xlarge"):
            fanout.prepare_manifest(
                matrix={
                    "include": [
                        self.matrix_entry(
                            "ec2-performance-1", machine="m5.xlarge"
                        )
                    ]
                },
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )

        proxy = self.matrix_entry(
            "ec2-proxy-interop-1",
            resource="ec2-proxy-interop",
            machine="m5.large",
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
        with self.assertRaisesRegex(fanout.FanoutError, "must use a 100 GB root volume"):
            fanout.prepare_manifest(
                matrix={"include": [proxy]},
                candidate=self.candidate,
                environment_id="release-environment",
                run_id="1",
                run_attempt="1",
            )

        long_soak = self.matrix_entry(
            "ec2-performance-soak-long-1",
            resource="ec2-performance-soak-long",
            machine="m5.2xlarge",
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
        long_soak["machine_type"] = "m5.xlarge"
        with self.assertRaisesRegex(fanout.FanoutError, "must use m5.2xlarge"):
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
            for state in sorted(fanout.TERMINAL_STATES):
                with self.subTest(state=state):
                    states[bounded["name"]] = state
                    decision = fanout.early_failure_decision(
                        manifest=manifest, downloads=downloads, states=states
                    )
                    self.assertTrue(decision["should_stop"])
                    self.assertEqual(decision["failed_shards"], ["bounded"])
            states[bounded["name"]] = "running"
            decision = fanout.early_failure_decision(
                manifest=manifest, downloads=downloads, states=states
            )
            self.assertFalse(decision["should_stop"])
            self.assertEqual(decision["early_settled"], 0)

    def test_instance_state_csv_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "states.csv"
            path.write_text("worker-1,running\nworker-2,terminated\n")
            self.assertEqual(
                fanout.load_instance_states(path),
                {"worker-1": "running", "worker-2": "terminated"},
            )
            path.write_text("worker-1,RUNNING,extra\n")
            with self.assertRaisesRegex(fanout.FanoutError, "invalid EC2"):
                fanout.load_instance_states(path)

    def test_user_data_env_file_single_quotes_every_value(self) -> None:
        env = {
            "RVOIP_PREBUILT_URI": "s3://bucket/key",
            "RVOIP_CANDIDATE": "it's 'quoted'",
            "RVOIP_EMPTY": "",
        }
        rendered = fanout.render_env_file(env)
        self.assertEqual(
            rendered,
            "RVOIP_CANDIDATE='it'\\''s '\\''quoted'\\'''\n"
            "RVOIP_EMPTY=''\n"
            "RVOIP_PREBUILT_URI='s3://bucket/key'\n",
        )
        # Every line is a valid shell assignment that round-trips the value.
        for key, value in env.items():
            with self.subTest(key=key):
                self.assertEqual(
                    subprocess.run(
                        ["bash", "-c", f'{rendered}printf %s "${key}"'],
                        capture_output=True,
                        check=True,
                        text=True,
                    ).stdout,
                    value,
                )
        script = fanout.render_user_data_script(
            startup="#!/bin/bash\necho startup\n", shutdown=None, env=env
        )
        self.assertTrue(script.startswith("#!/bin/bash\n"))
        self.assertIn("set -Eeuo pipefail", script)
        self.assertIn("chmod 0600 /etc/rvoip-release.env", script)
        self.assertIn("chmod 0755 /usr/local/lib/rvoip-release/startup.sh", script)
        self.assertTrue(script.endswith("exec /usr/local/lib/rvoip-release/startup.sh\n"))
        self.assertNotIn("rvoip-release-shutdown.service", script)
        self.assertNotIn("shutdown.sh", script)
        with self.assertRaisesRegex(fanout.FanoutError, "must match RVOIP_"):
            fanout.render_env_file({"PATH": "/bin"})
        with self.assertRaisesRegex(fanout.FanoutError, "must match RVOIP_"):
            fanout.render_env_file({"RVOIP_lower": "x"})
        with self.assertRaisesRegex(fanout.FanoutError, "newline"):
            fanout.render_env_file({"RVOIP_X": "a\nb"})
        with self.assertRaisesRegex(fanout.FanoutError, "KEY=VALUE"):
            fanout.parse_env_assignment("RVOIP_X")
        self.assertEqual(
            fanout.parse_env_assignment("RVOIP_X=a=b"), ("RVOIP_X", "a=b")
        )

    def test_user_data_installs_shutdown_checkpoint_unit(self) -> None:
        script = fanout.render_user_data_script(
            startup="echo startup\n",
            shutdown="echo shutdown\n",
            env={"RVOIP_RUN_ID": "1-1"},
        )
        self.assertIn("chmod 0755 /usr/local/lib/rvoip-release/shutdown.sh", script)
        unit_start = script.index("/etc/systemd/system/rvoip-release-shutdown.service")
        unit = script[unit_start:]
        for line in (
            "[Unit]",
            "Description=rvoip release shutdown checkpoint",
            "DefaultDependencies=no",
            "Before=shutdown.target",
            "[Service]",
            "Type=oneshot",
            "RemainAfterExit=yes",
            "ExecStart=/bin/true",
            "ExecStop=/usr/local/lib/rvoip-release/shutdown.sh",
            "TimeoutStopSec=180",
            "[Install]",
            "WantedBy=multi-user.target",
            "systemctl daemon-reload",
            "systemctl enable --now rvoip-release-shutdown.service",
        ):
            self.assertIn(line, unit)
        self.assertLess(
            script.index("systemctl enable --now"),
            script.index("exec /usr/local/lib/rvoip-release/startup.sh"),
        )
        # Heredocs are quoted with one delimiter that no embedded line can close.
        delimiters = {
            line.split("<<'", 1)[1].rstrip("'")
            for line in script.splitlines()
            if "<<'" in line
        }
        self.assertEqual(len(delimiters), 1)
        delimiter = delimiters.pop()
        self.assertTrue(delimiter.startswith("RVOIP_USER_DATA_"))
        self.assertEqual(script.count("<<'" + delimiter + "'"), 4)
        # The delimiter is content-derived, so user-data is reproducible, and a
        # script line that already spells it forces a different delimiter.
        self.assertEqual(
            fanout.heredoc_delimiter("echo a\n", "echo b\n"),
            fanout.heredoc_delimiter("echo a\n", "echo b\n"),
        )
        collided = fanout.heredoc_delimiter(delimiter + "\n")
        self.assertNotEqual(collided, delimiter)
        self.assertNotIn(collided, (delimiter + "\n").splitlines())
        base = fanout.heredoc_delimiter("x\n")
        self.assertEqual(fanout.heredoc_delimiter("x\n", "y\n"), fanout.heredoc_delimiter("x\n", "y\n"))
        self.assertTrue(base.startswith("RVOIP_USER_DATA_"))

    def test_user_data_round_trips_through_gzip_and_enforces_the_limit(self) -> None:
        startup = "#!/bin/bash\n" + "echo startup line\n" * 50
        env = {"RVOIP_GATES_B64": "cGVyZi5vbmU="}
        payload = fanout.build_user_data(startup=startup, shutdown=None, env=env)
        self.assertEqual(payload[:2], b"\x1f\x8b")
        self.assertLessEqual(len(payload), fanout.USER_DATA_MAX_BYTES)
        self.assertEqual(
            gzip.decompress(payload).decode(),
            fanout.render_user_data_script(startup=startup, shutdown=None, env=env),
        )
        incompressible = base64.b64encode(os.urandom(20000)).decode() + "\n"
        with self.assertRaisesRegex(fanout.FanoutError, "above the EC2 limit"):
            fanout.build_user_data(startup=incompressible, shutdown=None, env=env)

    def test_user_data_command_writes_compressed_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "startup.sh").write_text("echo start\n")
            (root / "shutdown.sh").write_text("echo stop\n")
            output = root / "out" / "user-data.gz"
            status = fanout.main(
                [
                    "user-data",
                    "--startup",
                    str(root / "startup.sh"),
                    "--shutdown",
                    str(root / "shutdown.sh"),
                    "--env",
                    "RVOIP_SHARD_ID=ec2-performance-1",
                    "--env",
                    "RVOIP_PREBUILT_URI=",
                    "--output",
                    str(output),
                ]
            )
            self.assertEqual(status, 0)
            script = gzip.decompress(output.read_bytes()).decode()
            self.assertIn("RVOIP_SHARD_ID='ec2-performance-1'", script)
            self.assertIn("RVOIP_PREBUILT_URI=''", script)
            self.assertIn("echo stop", script)
            self.assertNotEqual(
                fanout.main(
                    [
                        "user-data",
                        "--startup",
                        str(root / "startup.sh"),
                        "--env",
                        "HOME=/root",
                        "--output",
                        str(root / "bad.gz"),
                    ]
                ),
                0,
            )
            self.assertFalse((root / "bad.gz").exists())

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
                        / "_ec2-controller"
                        / worker["id"]
                        / "result.json"
                    ).is_file()
                )
            fanout_receipt = json.loads(
                (output / "_ec2-controller" / "fanout-receipt.json").read_text()
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
                "release-shard/_ec2-controller/injected.json",
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
