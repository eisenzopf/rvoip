#!/usr/bin/env python3
"""Fixture tests for exact Cargo performance executable resolution."""

import hashlib
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


artifact = load_module(
    "test_perf_cargo_artifact_impl", SCRIPT_DIR / "perf_cargo_artifact.py"
)


class CargoArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        (self.workspace / ".gitignore").write_text("/target\n", encoding="utf-8")
        self.test_source = self.workspace / "tests" / "perf_fixture.rs"
        self.test_source.parent.mkdir()
        self.test_source.write_text("#[test] fn fixture() {}\n", encoding="utf-8")
        subprocess.run(["git", "init", "-q"], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "config", "user.email", "fixture@example.invalid"],
            cwd=self.workspace,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Fixture"],
            cwd=self.workspace,
            check=True,
        )
        subprocess.run(["git", "add", "."], cwd=self.workspace, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "fixture"], cwd=self.workspace, check=True
        )
        self.binary = self.workspace / "target" / "release" / "deps" / "perf_fixture-1"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"current cargo executable")
        self.binary.chmod(0o755)
        self.messages = self.root / "cargo.jsonl"
        self.source_at_build = self.root / "source-at-build.json"
        artifact.write_source_provenance(self.workspace, self.source_at_build)

    def tearDown(self):
        self.temp.cleanup()

    def compiler_artifact(
        self,
        executable=None,
        *,
        name="perf_fixture",
        source=None,
        fresh=False,
    ):
        return {
            "reason": "compiler-artifact",
            "package_id": "path+file:///fixture#rvoip-sip@0.1.0",
            "target": {
                "kind": ["test"],
                "crate_types": ["bin"],
                "name": name,
                "src_path": str(source or self.test_source),
            },
            "profile": {
                "opt_level": "3",
                "debuginfo": 2,
                "debug_assertions": False,
                "overflow_checks": False,
                "test": True,
            },
            "executable": str(executable or self.binary),
            "fresh": fresh,
        }

    def write_messages(self, *messages):
        with self.messages.open("w", encoding="utf-8") as stream:
            stream.write("ordinary wrapper output is ignored\n")
            for message in messages:
                stream.write(json.dumps(message) + "\n")

    def test_exact_json_artifact_is_resolved_and_attested(self):
        # Cargo can repeat the same compiler artifact; identical paths do not
        # make the selection ambiguous.
        message = self.compiler_artifact(fresh=True)
        self.write_messages(message, message)
        manifest_path = self.root / "artifact.json"

        selected = artifact.write_artifact_manifest(
            messages_path=self.messages,
            manifest_path=manifest_path,
            workspace_root=self.workspace,
            source_at_build_path=self.source_at_build,
            expected_name="perf_fixture",
            expected_source=self.test_source,
            package="rvoip-sip",
            profile="release",
            features="perf-tests,perf-media-diagnostics",
            default_features=True,
        )

        self.assertEqual(selected, self.binary.resolve())
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest["schema"], artifact.MANIFEST_SCHEMA)
        self.assertEqual(manifest["executable"], str(self.binary.resolve()))
        self.assertEqual(
            manifest["executable_sha256"],
            hashlib.sha256(self.binary.read_bytes()).hexdigest(),
        )
        self.assertEqual(
            manifest["cargo_invocation"]["features_requested"],
            ["perf-tests", "perf-media-diagnostics"],
        )
        self.assertTrue(manifest["cargo_invocation"]["default_features"])
        self.assertTrue(manifest["cargo_artifact"]["fresh"])

    def test_unrelated_targets_are_ignored(self):
        self.write_messages(
            self.compiler_artifact(name="some_other_test"),
            self.compiler_artifact(),
        )
        selected, _ = artifact.resolve_cargo_test_artifact(
            self.messages, "perf_fixture", self.test_source
        )
        self.assertEqual(selected, self.binary.resolve())

    def test_missing_artifact_is_rejected(self):
        self.write_messages(self.compiler_artifact(name="other"))
        with self.assertRaisesRegex(artifact.ArtifactResolutionError, "exactly one"):
            artifact.resolve_cargo_test_artifact(
                self.messages, "perf_fixture", self.test_source
            )

    def test_ambiguous_artifacts_are_rejected(self):
        second = self.binary.with_name("perf_fixture-2")
        second.write_bytes(b"different feature variant")
        second.chmod(0o755)
        self.write_messages(
            self.compiler_artifact(), self.compiler_artifact(executable=second)
        )
        with self.assertRaisesRegex(artifact.ArtifactResolutionError, "exactly one"):
            artifact.resolve_cargo_test_artifact(
                self.messages, "perf_fixture", self.test_source
            )

    def test_non_executable_artifact_is_rejected(self):
        self.binary.chmod(0o644)
        self.write_messages(self.compiler_artifact())
        with self.assertRaisesRegex(artifact.ArtifactResolutionError, "not executable"):
            artifact.resolve_cargo_test_artifact(
                self.messages, "perf_fixture", self.test_source
            )

    def test_wrong_source_path_is_rejected(self):
        wrong_source = self.workspace / "tests" / "wrong.rs"
        wrong_source.write_text("// wrong target\n", encoding="utf-8")
        self.write_messages(self.compiler_artifact(source=wrong_source))
        with self.assertRaisesRegex(artifact.ArtifactResolutionError, "exactly one"):
            artifact.resolve_cargo_test_artifact(
                self.messages, "perf_fixture", self.test_source
            )

    def test_source_fingerprint_fence_detects_changes(self):
        unchanged = self.root / "unchanged.json"
        artifact.write_source_provenance(self.workspace, unchanged)
        expected = artifact.assert_same_source(
            self.source_at_build, unchanged, "during build"
        )
        self.assertEqual(
            expected,
            json.loads(self.source_at_build.read_text())["source_fingerprint_sha256"],
        )

        self.test_source.write_text("#[test] fn changed() {}\n", encoding="utf-8")
        changed = self.root / "changed.json"
        artifact.write_source_provenance(self.workspace, changed)
        with self.assertRaisesRegex(artifact.ArtifactResolutionError, "source changed"):
            artifact.assert_same_source(self.source_at_build, changed, "during run")


if __name__ == "__main__":
    unittest.main()
