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


if __name__ == "__main__":
    unittest.main()
