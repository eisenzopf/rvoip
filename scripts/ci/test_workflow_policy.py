from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
SHA_ACTION = re.compile(r"uses:\s*[^\s#]+@([0-9a-f]{40})(?:\s|#|$)")


class WorkflowPolicyTests(unittest.TestCase):
    def test_actions_are_immutable_and_prs_never_use_privileged_events(self) -> None:
        workflows = sorted((ROOT / ".github/workflows").glob("*.yml"))
        self.assertTrue(workflows)
        for workflow in workflows:
            text = workflow.read_text()
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("pull_request_target", text)
                uses = [line for line in text.splitlines() if "uses:" in line]
                self.assertTrue(all(SHA_ACTION.search(line) for line in uses))

    def test_pr_gate_has_no_secret_or_write_permission_surface(self) -> None:
        text = (ROOT / ".github/workflows/pr-gate.yml").read_text()
        self.assertNotIn("secrets.", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull_request_target", text)

    def test_main_release_tooling_installs_its_fuzz_toolchain(self) -> None:
        text = (ROOT / ".github/workflows/main-ci.yml").read_text()
        specialty = text.split("\n  specialty:\n", maxsplit=1)[1].split(
            "\n  main-gate:\n", maxsplit=1
        )[0]
        self.assertIn("Install nightly for release fuzz validation", specialty)
        self.assertIn("cargo-fuzz@0.13.2", specialty)
        self.assertGreaterEqual(specialty.count("matrix.gate == 'release-tooling'"), 2)

    def test_parallel_gcp_workspace_is_ephemeral_and_fail_closed(self) -> None:
        workflow = (ROOT / ".github/workflows/gcp-qualification-pilot.yml").read_text()
        startup = (ROOT / "infra/release-runners/gcp-pilot-startup.sh").read_text()

        self.assertIn("workspace-parallel", workflow)
        self.assertIn("--boot-disk-auto-delete", workflow)
        self.assertIn("Delete shard worker and attached disk", workflow)
        self.assertIn("Sweep workers left by interrupted shard controllers", workflow)
        self.assertIn("parallel GCP workers remain after cleanup", workflow)
        self.assertIn("if: always() && steps.worker.outputs.name != ''", workflow)
        self.assertIn("expected_shards", workflow)
        self.assertIn("expected_packages", workflow)
        self.assertIn("publishing_attempted == false", workflow)
        self.assertIn('"publishing_attempted": False', startup)
        for profile in (
            "workspace-policy",
            "workspace-shard-test",
            "workspace-shard-clippy",
            "workspace-doctest",
            "workspace-security-timing",
        ):
            self.assertIn(profile, startup)


if __name__ == "__main__":
    unittest.main()
