#!/usr/bin/env python3
"""Tests for per-PR change fragments and release-notes assembly."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("rvoip_release_notes", ROOT / "scripts/release/notes.py")
assert SPEC and SPEC.loader
notes = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = notes
SPEC.loader.exec_module(notes)


CHANGELOG = """# Changelog

## Unreleased

### Highlights

Hand-written highlight prose stays.

### Fixed

- An entry written by hand. (#400)

## 0.4.0 — 2026-11-01

- Old entry.
"""

NOTES = """# rvoip 9.9.9 Release Notes

Prose.

<!-- generated:changes:start -->
<!-- generated:changes:end -->

## Compatibility

Text.

<!-- generated:qualification:start -->
## Qualification record

Placeholder written before qualification.
<!-- generated:qualification:end -->
"""


def fragment(section: str, body: str, breaking: bool = False) -> str:
    return f"---\nsection: {section}\nbreaking: {str(breaking).lower()}\n---\n{body}\n"


class ReleaseNotesTests(unittest.TestCase):
    def tree(self, directory: str, fragments: dict[str, str]) -> Path:
        root = Path(directory)
        (root / "changes").mkdir()
        (root / "changes/README.md").write_text("how to write fragments\n")
        for name, text in fragments.items():
            (root / "changes" / name).write_text(text)
        (root / "docs/release").mkdir(parents=True)
        (root / "docs/release/RELEASE_NOTES_NEXT.md").write_text(NOTES)
        (root / "CHANGELOG.md").write_text(CHANGELOG)
        return root

    def test_fragments_are_validated(self) -> None:
        cases = {
            "not-a-pr.md": fragment("Fixed", "x"),
            "12.md": "no front matter\n",
            "13.md": fragment("Misc", "x"),
            "14.md": "---\nsection: Fixed\nbreaking: maybe\n---\nx\n",
            "15.md": fragment("Fixed", ""),
            "16.md": fragment("Fixed", "### a heading"),
            "17.md": fragment("Changed", "Renames a field.", breaking=True),
            "18.md": "---\nsection: Fixed\nowner: me\n---\nx\n",
        }
        for name, text in cases.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = self.tree(directory, {name: text})
                with self.assertRaises(notes.NotesError):
                    notes.load_fragments(root)
        with tempfile.TemporaryDirectory() as directory:
            root = self.tree(
                directory,
                {
                    "21-tls.md": fragment("Fixed", "TLS flows apply back-pressure."),
                    "22.md": fragment("Breaking changes", "Removes `Foo`. Migration: use `Bar`."),
                },
            )
            loaded = notes.load_fragments(root)
            self.assertEqual([item["pr"] for item in loaded], [22, 21])
            self.assertTrue(loaded[0]["breaking"])
            self.assertEqual(notes.check(root), 0)

    def test_assembly_appends_to_changelog_and_notes_then_removes_fragments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.tree(
                directory,
                {
                    "21.md": fragment("Fixed", "TLS flows apply back-pressure\ninstead of dropping responses."),
                    "22.md": fragment("Added", "`MediaExecutorConfig::v2()`."),
                    "23.md": fragment("Changed", "Default changed. Migration: set `x = false`.", breaking=True),
                },
            )
            self.assertEqual(notes.assemble(root, "9.9.9", None, None), 0)
            changelog = (root / "CHANGELOG.md").read_text()
            unreleased = changelog.split("## 0.4.0", 1)[0]
            self.assertIn("Hand-written highlight prose stays.", unreleased)
            self.assertIn(
                "- An entry written by hand. (#400)\n\n- TLS flows apply back-pressure\n"
                "  instead of dropping responses. (#21)",
                unreleased,
            )
            self.assertIn("### Added\n\n- `MediaExecutorConfig::v2()`. (#22)", unreleased)
            self.assertIn("### Changed\n\n- Default changed. Migration: set `x = false`. (#23)", unreleased)
            self.assertNotIn("(#21)", changelog.split("## 0.4.0", 1)[1])
            rendered = (root / "docs/release/RELEASE_NOTES_NEXT.md").read_text()
            self.assertIn("## Changes in this release", rendered)
            self.assertIn("### Breaking changes summary\n\n- Default changed.", rendered)
            self.assertIn("### Fixed\n\n- TLS flows", rendered)
            # Without evidence the qualification record is left for the author.
            self.assertIn("Placeholder written before qualification.", rendered)
            self.assertEqual(sorted(path.name for path in (root / "changes").iterdir()), ["README.md"])

    def test_qualification_record_comes_from_the_candidate_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.tree(directory, {})
            evidence = root / "evidence"
            (evidence / "release-qualification").mkdir(parents=True)
            (evidence / "release-evidence/interop.jambonz-latest").mkdir(parents=True)
            (evidence / "release-qualification/aggregate.json").write_text(
                json.dumps(
                    {
                        "status": "PASS",
                        "failures": [],
                        "profile": "remote-release",
                        "candidate_sha": "c" * 40,
                        "gate_count": 230,
                        "fresh_count": 225,
                        "reused_count": 5,
                    }
                )
            )
            (evidence / "release-evidence/interop.jambonz-latest/latest.json").write_text(
                json.dumps({"selected_release_line": "0.9.14"})
            )
            notes.assemble(root, "9.9.9", evidence, "https://example.invalid/run/1")
            rendered = (root / "docs/release/RELEASE_NOTES_NEXT.md").read_text()
            self.assertNotIn("Placeholder", rendered)
            record = rendered.split(notes.QUALIFICATION_START, 1)[1]
            for expected in (
                "## Qualification record",
                "c" * 40,
                "230 of 230 gates",
                "225 run fresh, 5 reused",
                "Jambonz OSS was tested at the pinned `0.9.14`",
                "protected",
                "`v9.9.9` tag",
                "Performance evaluation",
            ):
                self.assertIn(expected, record)
            for forbidden in ("pending", "Candidate Release Notes"):
                self.assertNotIn(forbidden, record)

    def test_failed_qualification_cannot_fill_the_record(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.tree(directory, {})
            evidence = root / "evidence/release-qualification"
            evidence.mkdir(parents=True)
            (evidence / "aggregate.json").write_text(
                json.dumps({"status": "FAIL", "failures": ["x"], "profile": "remote-release"})
            )
            with self.assertRaises(notes.NotesError):
                notes.assemble(root, "9.9.9", root / "evidence", "https://example.invalid")

    def test_notes_must_keep_one_of_each_generated_block(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.tree(directory, {})
            path = root / "docs/release/RELEASE_NOTES_NEXT.md"
            path.write_text(NOTES.replace(notes.CHANGES_END, ""))
            with self.assertRaises(notes.NotesError):
                notes.check(root)

    def test_repository_notes_and_fragments_are_valid(self) -> None:
        self.assertEqual(notes.check(ROOT), 0)


if __name__ == "__main__":
    unittest.main()
