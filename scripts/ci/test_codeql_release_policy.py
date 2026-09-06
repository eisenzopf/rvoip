#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.ci.check_codeql_release_policy import DEFAULT_CATEGORIES, evaluate


def analyses(candidate: str):
    return [
        {
            "id": index,
            "commit_sha": candidate,
            "category": category,
            "tool": {"name": "CodeQL"},
        }
        for index, category in enumerate(DEFAULT_CATEGORIES, 1)
    ]


class CodeqlReleasePolicyTests(unittest.TestCase):
    def test_exact_candidate_with_zero_open_alerts_passes(self):
        result = evaluate(analyses("a" * 40), [], "a" * 40)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["open_alert_count"], 0)

    def test_stale_or_missing_language_fails(self):
        candidate = "b" * 40
        values = analyses(candidate)[1:]
        values[0]["commit_sha"] = "c" * 40
        result = evaluate(values, [], candidate)
        self.assertEqual(result["status"], "FAIL")
        self.assertTrue(any("missing" in failure for failure in result["failures"]))
        self.assertTrue(any("not bound" in failure for failure in result["failures"]))

    def test_every_open_alert_blocks_regardless_of_severity(self):
        candidate = "d" * 40
        result = evaluate(
            analyses(candidate),
            [{"number": 42, "state": "open", "rule": {"security_severity_level": "low"}}],
            candidate,
        )
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["open_alert_numbers"], [42])


if __name__ == "__main__":
    unittest.main()
