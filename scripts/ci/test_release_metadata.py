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


if __name__ == "__main__":
    unittest.main()
