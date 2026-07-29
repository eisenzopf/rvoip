#!/usr/bin/env python3
"""Tests for the unified workspace release implementation."""

from __future__ import annotations

import importlib.util
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
                        "schema": "rvoip-unified-release-verification-v3",
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
                            "version": "0.3.2",
                            "disposition": "APPROVED-WITH-EXCEPTION",
                            "strict_automated_status": "NON-RC",
                        }
                    }
                )
            )
            log = mock.Mock()
            qualification = release.verify_beta_reporting(
                root,
                "0.3.2",
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
            self.assertEqual(command[-2:], ["--version", "0.3.2"])

    def test_strict_and_exception_beta_inputs_are_mutually_exclusive(self) -> None:
        with self.assertRaises(release.ReleaseError):
            release.verify_beta_reporting(
                Path("/repo"),
                "0.3.2",
                "/tmp/strict-report",
                "/tmp/exception.json",
                mock.Mock(),
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
            root, "0.3.2", locked=True
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
