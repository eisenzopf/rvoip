#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.ci.check_codeql_release_policy import (
    DEFAULT_CATEGORIES,
    evaluate,
    same_tree_commits,
)


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

    def test_an_older_main_commit_stays_qualified_after_newer_analyses(self):
        candidate = "e" * 40
        newer = analyses("f" * 40)
        result = evaluate(newer + analyses(candidate), [], candidate)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(
            set(result["analysis_ids"]), set(DEFAULT_CATEGORIES)
        )

    def test_every_open_alert_blocks_regardless_of_severity(self):
        candidate = "d" * 40
        result = evaluate(
            analyses(candidate),
            [{"number": 42, "state": "open", "rule": {"security_severity_level": "low"}}],
            candidate,
        )
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["open_alert_numbers"], [42])


    def test_a_merge_commit_with_the_candidates_tree_carries_its_analysis(self):
        # 0.5.0: the version-bump candidate reached main through a merge
        # commit whose tree is the candidate's, so main's push analysis is
        # bound to the merge commit and never to the candidate itself.
        candidate, merge = "1" * 40, "2" * 40
        main = analyses(merge)
        self.assertEqual(evaluate(main, [], candidate)["status"], "FAIL")
        result = evaluate(main, [], candidate, same_tree_commits=frozenset({merge}))
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(
            set(result["analysis_commits"].values()), {merge}
        )
        self.assertEqual(result["same_tree_commits"], [merge])

    def test_the_candidates_own_analysis_wins_over_a_same_tree_commit(self):
        candidate, merge = "3" * 40, "4" * 40
        result = evaluate(
            analyses(merge) + analyses(candidate),
            [],
            candidate,
            same_tree_commits=frozenset({merge}),
        )
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(set(result["analysis_commits"].values()), {candidate})

    def test_same_tree_commits_compares_trees_newest_first_within_a_limit(self):
        candidate = "5" * 40
        trees = {candidate: "tree-a", "6" * 40: "tree-a", "7" * 40: "tree-b", "8" * 40: "tree-a"}
        main = analyses("6" * 40) + analyses("7" * 40) + analyses(candidate) + analyses("8" * 40)
        asked: list[str] = []

        def tree_of(commit: str) -> str:
            asked.append(commit)
            return trees[commit]

        self.assertEqual(
            same_tree_commits(main, candidate, tree_of),
            frozenset({"6" * 40, "8" * 40}),
        )
        self.assertNotIn(candidate, asked[1:])
        self.assertEqual(
            same_tree_commits(main, candidate, tree_of, limit=2),
            frozenset({"6" * 40}),
        )

if __name__ == "__main__":
    unittest.main()
