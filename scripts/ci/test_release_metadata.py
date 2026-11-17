#!/usr/bin/env python3
"""Repository-level checks for active release metadata."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("rvoip_release", ROOT / "scripts/release.py")
assert SPEC is not None and SPEC.loader is not None
RELEASE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RELEASE
SPEC.loader.exec_module(RELEASE)


class ActiveReleaseMetadataTests(unittest.TestCase):
    def test_active_metadata_matches_workspace_version(self) -> None:
        with (ROOT / "Cargo.toml").open("rb") as handle:
            version = tomllib.load(handle)["workspace"]["package"]["version"]
        RELEASE.validate_active_release_metadata(ROOT, version)

    def test_release_notes_keep_high_cps_as_a_non_claim(self) -> None:
        # Moved from rvoip-sip's beta_release_docs.rs when the notes moved to
        # docs/release/: the notes are release bookkeeping, outside every
        # qualification gate's inputs, so their content is policed here, in
        # the PR Gate's universal policy job, on every pull request.
        notes = (ROOT / "docs/release/RELEASE_NOTES_NEXT.md").read_text(encoding="utf-8")
        self.assertIn("General-user 10,000 CPS full-media capability", notes)
        stub = (ROOT / "crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("docs/release/RELEASE_NOTES_NEXT.md", stub)


if __name__ == "__main__":
    unittest.main()
