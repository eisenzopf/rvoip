#!/usr/bin/env python3
"""Tests for the unified workspace release implementation."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("release.py")
SPEC = importlib.util.spec_from_file_location("rvoip_release", SCRIPT)
assert SPEC and SPEC.loader
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def package(name: str, version: str, dependencies: list[dict] | None = None) -> dict:
    return {
        "name": name,
        "version": version,
        "dependencies": dependencies or [],
        "manifest_path": f"/tmp/{name}/Cargo.toml",
        "id": name,
        "publish": None,
    }


def dependency(name: str, kind: str | None = None) -> dict:
    return {"name": name, "kind": kind}


def targeted_attestation_fixture(
    root: Path,
) -> tuple[Path, str, str, dict[str, object]]:
    release.run(["git", "init", "-b", "main"], cwd=root)
    release.run(["git", "config", "user.name", "Release Test"], cwd=root)
    release.run(
        ["git", "config", "user.email", "release-test@example.invalid"],
        cwd=root,
    )
    changed = root / "docs/PRD.md"
    changed.parent.mkdir(parents=True)
    changed.write_text("base\n")
    changed_path = changed.relative_to(root).as_posix()
    release.run(["git", "add", changed_path], cwd=root)
    release.run(["git", "commit", "-m", "base"], cwd=root)
    base = release.git_output(root, "rev-parse", "HEAD")
    release.run(["git", "tag", release.TARGETED_DELTA_BASE_TAG, base], cwd=root)
    changed.write_text("release\n")
    release.run(["git", "add", changed_path], cwd=root)
    release.run(["git", "commit", "-m", "release"], cwd=root)
    head = release.git_output(root, "rev-parse", "HEAD")

    postgres_evidence = root / "postgres-live.json"
    postgres_payload = {
        "schema": release.TARGETED_POSTGRES_EVIDENCE_SCHEMA,
        "git_commit": head,
        "argv": list(release.TARGETED_POSTGRES_COMMAND),
        "exit_status": 0,
        "live_database": True,
        "ephemeral_database": True,
        "server_version": "PostgreSQL 17.2",
        "environment": {
            "provider": "temporary-test-service",
            "image": "postgres:17.2",
            "database": "rvoip_vcon_test",
            "run_id": "release-test-001",
        },
        "recorded_at": "2026-07-29T00:00:00Z",
    }
    postgres_evidence.write_text(json.dumps(postgres_payload))
    payload: dict[str, object] = {
        "schema": release.TARGETED_DELTA_ATTESTATION_SCHEMA,
        "release": {
            "version": "0.3.3",
            "git_commit": head,
            "base_commit": base,
            "vcon_schema_commit": release.VCON_SCHEMA_COMMIT,
        },
        "allowed_changed_paths": [changed_path],
        "commands": [
            {
                "name": name,
                "argv": list(argv),
                "exit_status": 0,
                "git_commit": head,
            }
            for name, argv in release.TARGETED_DELTA_COMMANDS
        ],
        "postgresql": {
            "live_database": True,
            "ephemeral_database": True,
            "server_version": "PostgreSQL 17.2",
            "environment": {
                "provider": "temporary-test-service",
                "image": "postgres:17.2",
                "database": "rvoip_vcon_test",
                "run_id": "release-test-001",
            },
            "command": {
                "name": "postgres-core-store-live",
                "argv": list(release.TARGETED_POSTGRES_COMMAND),
                "exit_status": 0,
                "git_commit": head,
            },
            "evidence": {
                "path": postgres_evidence.name,
                "sha256": release.hashlib.sha256(
                    postgres_evidence.read_bytes()
                ).hexdigest(),
            },
        },
        "approval": {
            "approved_by": "project-owner",
            "approved_at": "2026-07-29T00:00:00Z",
            "rationale": "The release delta is limited to the approved vCon fixes.",
        },
    }
    attestation = root / "targeted-delta.json"
    attestation.write_text(json.dumps(payload))
    return attestation, base, head, payload


class ReleaseTests(unittest.TestCase):
    def test_semver_requires_stable_three_part_version(self) -> None:
        self.assertEqual(release.require_version("0.3.0"), "0.3.0")
        for value in ("0.3", "v0.3.0", "0.3.0-beta.1", "next"):
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                release.require_version(value)

    def test_topological_order_includes_0_3_and_ignores_dev_cycle(self) -> None:
        packages = {
            "core": package(
                "core", "0.3.0", [dependency("facade", "dev")]
            ),
            "transport": package(
                "transport", "0.3.0", [dependency("core")]
            ),
            "facade": package(
                "facade", "0.3.0", [dependency("transport")]
            ),
        }
        self.assertEqual(
            release.topological_order(packages),
            ["core", "transport", "facade"],
        )

    def test_normal_dependency_cycle_fails_closed(self) -> None:
        packages = {
            "a": package("a", "0.3.0", [dependency("b")]),
            "b": package("b", "0.3.0", [dependency("a")]),
        }
        with self.assertRaises(release.ReleaseError):
            release.topological_order(packages)

    def test_workspace_package_discovery_rejects_duplicates(self) -> None:
        metadata = {
            "workspace_members": ["first", "second"],
            "packages": [
                {**package("same", "0.3.0"), "id": "first"},
                {**package("same", "0.3.0"), "id": "second"},
            ],
        }
        with self.assertRaises(release.ReleaseError):
            release.publishable_packages(metadata)

    def test_manifest_version_migration_is_section_scoped(self) -> None:
        source = """[package]
name = "demo"
version = "0.1.3"

[dependencies]
other = { version = "7.5" }
"""
        expected = """[package]
name = "demo"
version.workspace = true

[dependencies]
other = { version = "7.5" }
"""
        self.assertEqual(
            release.replace_section_version(
                source, "package", "version.workspace = true"
            ),
            expected,
        )

    def test_workspace_dependency_update_only_changes_internal_entries(self) -> None:
        source = """[workspace.dependencies]
rvoip-a = { path = "a", version = "0.1.3" }
serde = { version = "1.0" }
"""
        updated = release.update_workspace_dependency_versions(
            source, {"rvoip-a"}, "0.3.0"
        )
        self.assertIn('rvoip-a = { path = "a", version = "0.3.0" }', updated)
        self.assertIn('serde = { version = "1.0" }', updated)

    def test_planned_version_edits_update_renamed_member_dependency(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rtc_manifest = root / "rvoip-rtc/Cargo.toml"
            stack_manifest = root / "rvoip-webrtc-stack/Cargo.toml"
            rtc_manifest.parent.mkdir()
            stack_manifest.parent.mkdir()
            (root / "Cargo.toml").write_text(
                """[workspace.package]
version = "0.3.3"

[workspace.dependencies]
rvoip-rtc = { path = "rvoip-rtc", version = "0.3.3" }
rvoip-webrtc-stack = { path = "rvoip-webrtc-stack", version = "0.3.3" }
"""
            )
            rtc_manifest.write_text(
                """[package]
name = "rvoip-rtc"
version.workspace = true
"""
            )
            stack_manifest.write_text(
                """[package]
name = "rvoip-webrtc-stack"
version.workspace = true

[dependencies.rtc]
version = "0.3.3"
path = "../rvoip-rtc"
package = "rvoip-rtc"

[dependencies.serde_json]
version = "1.0"
"""
            )
            packages = {
                "rvoip-rtc": {
                    **package("rvoip-rtc", "0.3.3"),
                    "manifest_path": str(rtc_manifest),
                },
                "rvoip-webrtc-stack": {
                    **package(
                        "rvoip-webrtc-stack",
                        "0.3.3",
                        [
                            {
                                "name": "rvoip-rtc",
                                "rename": "rtc",
                                "req": "^0.3.3",
                                "kind": None,
                            }
                        ],
                    ),
                    "manifest_path": str(stack_manifest),
                },
            }

            edits = release.planned_version_edits(root, packages, "0.3.4")

            self.assertIn(
                'rvoip-rtc = { path = "rvoip-rtc", version = "0.3.4" }',
                edits[root / "Cargo.toml"].decode(),
            )
            self.assertIn(
                '[dependencies.rtc]\nversion = "0.3.4"',
                edits[stack_manifest].decode(),
            )
            self.assertIn(
                '[dependencies.serde_json]\nversion = "1.0"',
                edits[stack_manifest].decode(),
            )

    def test_member_dependency_versions_reject_stale_renamed_requirement(
        self,
    ) -> None:
        packages = {
            "rvoip-rtc": package("rvoip-rtc", "0.3.3"),
            "rvoip-webrtc-stack": package(
                "rvoip-webrtc-stack",
                "0.3.3",
                [
                    {
                        "name": "rvoip-rtc",
                        "rename": "rtc",
                        "req": "^0.3.2",
                        "kind": None,
                    }
                ],
            ),
        }
        with self.assertRaisesRegex(
            release.ReleaseError,
            r"rvoip-webrtc-stack -> rtc \(rvoip-rtc\)@\^0\.3\.2",
        ):
            release.validate_member_dependency_versions(packages, "0.3.3")

    def test_member_dependency_versions_accept_current_and_path_only_dev(
        self,
    ) -> None:
        packages = {
            "rvoip-rtc": package("rvoip-rtc", "0.3.3"),
            "rvoip-webrtc-stack": package(
                "rvoip-webrtc-stack",
                "0.3.3",
                [
                    {
                        "name": "rvoip-rtc",
                        "rename": "rtc",
                        "req": "^0.3.3",
                        "kind": None,
                    },
                    {
                        "name": "rvoip-rtc",
                        "rename": None,
                        "req": "*",
                        "kind": "dev",
                    },
                ],
            ),
        }
        release.validate_member_dependency_versions(packages, "0.3.3")

    def test_checksum_safe_resume(self) -> None:
        release.assert_existing_checksum(
            "demo", "0.3.0", "abc", {"checksum": "abc"}
        )
        with self.assertRaises(release.ReleaseError):
            release.assert_existing_checksum(
                "demo", "0.3.0", "abc", {"checksum": "def"}
            )

    def test_package_file_manifest_is_hashed_and_rejects_unsafe_paths(self) -> None:
        class FakeLog:
            def __init__(self, output: str):
                self.output = output
                self.events: list[tuple[str, dict]] = []

            def command_capture(self, argv: list[str], cwd: Path) -> str:
                return self.output

            def event(self, kind: str, **values: object) -> None:
                self.events.append((kind, values))

        safe = FakeLog(
            ".cargo_vcs_info.json\nCargo.lock\nCargo.toml\n"
            "Cargo.toml.orig\nREADME.md\nsrc/lib.rs\n"
        )
        paths, digest = release.package_file_manifest(
            Path("/repo"), "demo", "0.3.0", safe
        )
        self.assertEqual(paths[-1], "src/lib.rs")
        self.assertEqual(len(digest), 64)
        self.assertEqual(safe.events[0][0], "package-file-manifest")

        unsafe = FakeLog("Cargo.toml\nCargo.toml.orig\n../secret\n")
        with self.assertRaises(release.ReleaseError):
            release.package_file_manifest(
                Path("/repo"), "demo", "0.3.0", unsafe
            )

    def test_verification_receipt_allows_deferred_archive_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt_path = (
                root / "target/release-logs/0.3.0/verification.json"
            )
            receipt_path.parent.mkdir(parents=True)
            receipt_path.write_text(
                json.dumps(
                    {
                        "schema": release.VERIFICATION_RECEIPT_SCHEMA,
                        "version": "0.3.0",
                        "git_commit": "abc",
                        "package_count": 2,
                        "ordered_packages": ["leaf", "dependent"],
                        "package_sha256": {"leaf": "a" * 64},
                        "package_file_manifest_sha256": {
                            "leaf": "b" * 64,
                            "dependent": "c" * 64,
                        },
                        "beta_qualification": {
                            "mode": "strict",
                            "disposition": "RELEASE-CANDIDATE",
                            "strict_automated_status": "PASS",
                        },
                        "verification_scope": {
                            "mode": "full",
                            "workspace_manifest": "PASS",
                            "workspace_compile": "PASS",
                            "workspace_tests": "PASS",
                            "workspace_doctests": "PASS",
                            "beta_suite": "PASS",
                            "targeted_commands": [],
                            "postgresql_evidence": None,
                            "package_file_manifests": "PASS",
                            "package_archives": (
                                "VERIFIED-WHEN-REGISTRY-RESOLVABLE"
                            ),
                        },
                    }
                )
            )
            receipt = release.read_verification_receipt(
                root, "0.3.0", "abc", ["leaf", "dependent"]
            )
            self.assertEqual(set(receipt["package_sha256"]), {"leaf"})

    def test_beta_exception_is_explicit_and_hash_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            attestation = root / "exception-attestation.json"
            attestation.write_text(
                json.dumps(
                    {
                        "release": {
                            "version": "0.3.3",
                            "disposition": "APPROVED-WITH-EXCEPTION",
                            "strict_automated_status": "NON-RC",
                        }
                    }
                )
            )
            log = mock.Mock()
            qualification = release.verify_beta_reporting(
                root,
                "0.3.3",
                None,
                str(attestation),
                log,
            )
            self.assertEqual(qualification["mode"], "owner-approved-exception")
            self.assertEqual(
                qualification["disposition"], "APPROVED-WITH-EXCEPTION"
            )
            self.assertEqual(qualification["strict_automated_status"], "NON-RC")
            self.assertEqual(
                qualification["attestation_sha256"],
                release.hashlib.sha256(attestation.read_bytes()).hexdigest(),
            )
            command = log.command.call_args.args[0]
            self.assertIn("release_exception_attestation.py", command[1])
            self.assertEqual(command[-2:], ["--version", "0.3.3"])

    def test_strict_and_exception_beta_inputs_are_mutually_exclusive(self) -> None:
        with self.assertRaises(release.ReleaseError):
            release.verify_beta_reporting(
                Path("/repo"),
                "0.3.3",
                "/tmp/strict-report",
                "/tmp/exception.json",
                mock.Mock(),
            )

    def test_targeted_delta_attestation_is_exact_and_commit_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            attestation, base, head, _ = targeted_attestation_fixture(root)
            qualification, commands = release.verify_targeted_delta_attestation(
                root,
                "0.3.3",
                head,
                str(attestation),
                mock.Mock(),
            )
            self.assertEqual(qualification["mode"], "targeted-delta")
            self.assertEqual(qualification["base_commit"], base)
            self.assertEqual(
                qualification["vcon_schema_commit"],
                release.VCON_SCHEMA_COMMIT,
            )
            self.assertEqual(
                [name for name, _ in commands],
                [name for name, _ in release.TARGETED_DELTA_COMMANDS],
            )
            self.assertEqual(
                qualification["postgresql"]["status"], "ATTESTED-PASS"
            )

    def test_targeted_postgres_command_opts_into_live_database_tests(self) -> None:
        self.assertIn(
            "core-store,live-tests",
            release.TARGETED_POSTGRES_COMMAND,
        )

    def test_targeted_delta_rejects_path_command_and_postgres_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            attestation, _, head, payload = targeted_attestation_fixture(root)
            mutations = {
                "changed path": lambda value: value.update(
                    {"allowed_changed_paths": ["other.txt"]}
                ),
                "missing base": lambda value: value["release"].update(
                    {"base_commit": "0" * 40}
                ),
                "schema commit": lambda value: value["release"].update(
                    {"vcon_schema_commit": "1" * 40}
                ),
                "command argv": lambda value: value["commands"][0].update(
                    {"argv": ["cargo", "test"]}
                ),
                "postgres commit": lambda value: value["postgresql"][
                    "command"
                ].update({"git_commit": "0" * 40}),
                "postgres not ephemeral": lambda value: value["postgresql"].update(
                    {"ephemeral_database": False}
                ),
            }
            for label, mutate in mutations.items():
                with self.subTest(label=label):
                    candidate = json.loads(json.dumps(payload))
                    mutate(candidate)
                    attestation.write_text(json.dumps(candidate))
                    with self.assertRaises(release.ReleaseError):
                        release.verify_targeted_delta_attestation(
                            root,
                            "0.3.3",
                            head,
                            str(attestation),
                            mock.Mock(),
                        )

    def test_targeted_delta_enforces_hard_path_policy_and_evidence_schema(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            attestation, _, head, payload = targeted_attestation_fixture(root)

            evidence_path = root / payload["postgresql"]["evidence"]["path"]
            evidence = json.loads(evidence_path.read_text())
            evidence["schema"] = "self-declared-schema"
            evidence_path.write_text(json.dumps(evidence))
            payload["postgresql"]["evidence"]["sha256"] = (
                release.hashlib.sha256(evidence_path.read_bytes()).hexdigest()
            )
            attestation.write_text(json.dumps(payload))
            with self.assertRaises(release.ReleaseError):
                release.verify_targeted_delta_attestation(
                    root, "0.3.3", head, str(attestation), mock.Mock()
                )

            unrelated = root / "crates/sip/unrelated.rs"
            unrelated.parent.mkdir(parents=True)
            unrelated.write_text("not part of the vCon delta\n")
            release.run(
                ["git", "add", unrelated.relative_to(root).as_posix()], cwd=root
            )
            release.run(["git", "commit", "-m", "unrelated"], cwd=root)
            new_head = release.git_output(root, "rev-parse", "HEAD")
            payload["release"]["git_commit"] = new_head
            payload["allowed_changed_paths"] = [
                "crates/sip/unrelated.rs",
                "docs/PRD.md",
            ]
            for command in payload["commands"]:
                command["git_commit"] = new_head
            payload["postgresql"]["command"]["git_commit"] = new_head
            evidence.update(
                {
                    "schema": release.TARGETED_POSTGRES_EVIDENCE_SCHEMA,
                    "git_commit": new_head,
                }
            )
            evidence_path.write_text(json.dumps(evidence))
            payload["postgresql"]["evidence"]["sha256"] = (
                release.hashlib.sha256(evidence_path.read_bytes()).hexdigest()
            )
            attestation.write_text(json.dumps(payload))
            with self.assertRaises(release.ReleaseError):
                release.verify_targeted_delta_attestation(
                    root, "0.3.3", new_head, str(attestation), mock.Mock()
                )

    def test_targeted_delta_allows_only_metadata_files_outside_vcon_scope(
        self,
    ) -> None:
        self.assertTrue(
            release.targeted_delta_path_allowed(
                "crates/webrtc/rvoip-webrtc-stack/Cargo.toml"
            )
        )
        self.assertTrue(
            release.targeted_delta_path_allowed(
                "crates/sip/rvoip-sip/Cargo.toml"
            )
        )
        self.assertFalse(
            release.targeted_delta_path_allowed(
                "crates/webrtc/rvoip-webrtc-stack/src/lib.rs"
            )
        )
        self.assertFalse(
            release.targeted_delta_path_allowed(
                "crates/sip/rvoip-sip/src/lib.rs"
            )
        )

    def test_targeted_delta_requires_timezone_aware_approval(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            attestation, _, head, payload = targeted_attestation_fixture(root)
            payload["approval"]["approved_at"] = "2026-07-29T00:00:00"
            attestation.write_text(json.dumps(payload))
            with self.assertRaises(release.ReleaseError):
                release.verify_targeted_delta_attestation(
                    root, "0.3.3", head, str(attestation), mock.Mock()
                )

    def test_targeted_delta_receipt_is_accepted_without_full_test_claims(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt_path = (
                root / "target/release-logs/0.3.3/verification.json"
            )
            receipt_path.parent.mkdir(parents=True)
            postgres = {
                "status": "ATTESTED-PASS",
                "live_database": True,
                "ephemeral_database": True,
                "server_version": "PostgreSQL 17.2",
                "environment": {
                    "provider": "temporary-test-service",
                    "image": "postgres:17.2",
                    "database": "rvoip_vcon_test",
                    "run_id": "release-test-001",
                },
                "recorded_at": "2026-07-29T00:00:00Z",
                "command": list(release.TARGETED_POSTGRES_COMMAND),
                "evidence_path": "postgres-live.log",
                "evidence_sha256": "d" * 64,
            }
            receipt_path.write_text(
                json.dumps(
                    {
                        "schema": release.VERIFICATION_RECEIPT_SCHEMA,
                        "version": "0.3.3",
                        "git_commit": "a" * 40,
                        "package_count": 1,
                        "ordered_packages": ["rvoip-vcon"],
                        "package_sha256": {},
                        "package_file_manifest_sha256": {
                            "rvoip-vcon": "b" * 64,
                        },
                        "beta_qualification": {
                            "mode": "targeted-delta",
                            "disposition": "APPROVED-TARGETED-DELTA",
                            "strict_automated_status": "NOT-RERUN",
                            "workspace_test_status": "TARGETED-ONLY",
                            "base_commit": "c" * 40,
                            "vcon_schema_commit": release.VCON_SCHEMA_COMMIT,
                            "changed_paths": ["docs/PRD.md"],
                            "changed_path_count": 1,
                            "targeted_command_count": len(
                                release.TARGETED_DELTA_COMMANDS
                            ),
                            "attestation_sha256": "e" * 64,
                            "postgresql": postgres,
                        },
                        "verification_scope": {
                            "mode": "targeted-delta",
                            "workspace_manifest": "PASS",
                            "workspace_compile": "PASS",
                            "workspace_tests": "NOT-RERUN",
                            "workspace_doctests": "NOT-RERUN",
                            "beta_suite": "NOT-RERUN",
                            "targeted_commands": [
                                {
                                    "name": name,
                                    "argv": list(argv),
                                    "exit_status": 0,
                                }
                                for name, argv in release.TARGETED_DELTA_COMMANDS
                            ],
                            "postgresql_evidence": postgres,
                            "package_file_manifests": "PASS",
                            "package_archives": (
                                "VERIFIED-WHEN-REGISTRY-RESOLVABLE"
                            ),
                        },
                    }
                )
            )
            receipt = release.read_verification_receipt(
                root, "0.3.3", "a" * 40, ["rvoip-vcon"]
            )
            self.assertEqual(
                receipt["verification_scope"]["workspace_tests"],
                "NOT-RERUN",
            )

            receipt["verification_scope"]["workspace_tests"] = "PASS"
            receipt_path.write_text(json.dumps(receipt))
            with self.assertRaises(release.ReleaseError):
                release.read_verification_receipt(
                    root, "0.3.3", "a" * 40, ["rvoip-vcon"]
                )

    def test_targeted_delta_cli_is_mutually_exclusive(self) -> None:
        parser = release.parser()
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(
                [
                    "verify",
                    "--version",
                    "0.3.3",
                    "--beta-report-root",
                    "strict",
                    "--targeted-delta-attestation",
                    "targeted.json",
                ]
            )

    def test_visibility_timeout_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            log = release.ReleaseLog(Path(directory), "0.3.0", "test")
            with (
                mock.patch.object(release, "crates_io_version", return_value=None),
                mock.patch.object(release.time, "monotonic", side_effect=[0, 2]),
                self.assertRaises(release.ReleaseError),
            ):
                release.wait_for_version(
                    "demo",
                    "0.3.0",
                    local_sha256="abc",
                    poll_seconds=0,
                    timeout_seconds=1,
                    log=log,
                )

    def test_dirty_release_state_fails_closed(self) -> None:
        with mock.patch.object(
            release, "git_output", return_value=" M Cargo.toml"
        ):
            with self.assertRaises(release.ReleaseError):
                release.ensure_release_state(
                    Path("/repo"), "0.3.0", require_no_tag=True
                )

    def test_off_main_release_state_fails_closed(self) -> None:
        with mock.patch.object(
            release, "git_output", side_effect=["", "feature"]
        ):
            with self.assertRaises(release.ReleaseError):
                release.ensure_release_state(
                    Path("/repo"), "0.3.0", require_no_tag=True
                )

    def test_stale_origin_main_fails_closed(self) -> None:
        completed = release.subprocess.CompletedProcess(
            ["git", "ls-remote"], 0, stdout="def\trefs/heads/main\n", stderr=""
        )
        with (
            mock.patch.object(
                release,
                "git_output",
                side_effect=["", "main", "abc"],
            ),
            mock.patch.object(release, "run", return_value=completed),
            self.assertRaises(release.ReleaseError),
        ):
            release.ensure_release_state(
                Path("/repo"), "0.3.0", require_no_tag=True
            )

    def test_current_workspace_has_all_44_unique_publishable_packages(self) -> None:
        root = SCRIPT.parent.parent
        packages, ordered = release.validate_workspace(
            root, "0.3.3", locked=True
        )
        self.assertEqual(len(packages), release.EXPECTED_PACKAGE_COUNT)
        self.assertEqual(len(ordered), 44)
        self.assertIn("rvoip-sip-dialog", packages)
        self.assertIn("rvoip-sip-transport", packages)
        self.assertIn("rvoip-moq", packages)
        self.assertIn("rvoip-moq-native", packages)
        self.assertIn("rvoip-moq-relay", packages)
        self.assertIn("rvoip-moq-transport", packages)
        self.assertIn("rvoip-rtc", packages)
        self.assertIn("rvoip-webrtc-stack", packages)
        self.assertIn("rvoip-vapi", packages)


if __name__ == "__main__":
    unittest.main()
