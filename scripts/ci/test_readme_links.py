from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ci/check_readme_links.py"
SPEC = importlib.util.spec_from_file_location("check_readme_links", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
readme_links = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(readme_links)


class ReadmeLinkPolicyTests(unittest.TestCase):
    def test_current_release_readmes_pass(self) -> None:
        self.assertEqual(readme_links.validate_local_links(ROOT), [])
        self.assertEqual(readme_links.validate_release_surfaces(ROOT), [])
        self.assertEqual(readme_links.validate_current_release_evidence(ROOT), [])

    def test_rvoip_sip_api_links_use_rustdoc_defining_module_paths(self) -> None:
        version = readme_links.workspace_version(ROOT)
        text = (ROOT / "crates/sip/rvoip-sip/README.md").read_text(encoding="utf-8")
        expected = (
            "api/endpoint/struct.Endpoint.html",
            "api/stream_peer/struct.StreamPeer.html",
            "api/callback_peer/struct.CallbackPeer.html",
            "api/unified/struct.UnifiedCoordinator.html",
            "api/handle/struct.SessionHandle.html",
        )
        for rustdoc_path in expected:
            with self.subTest(rustdoc_path=rustdoc_path):
                self.assertIn(
                    f"https://docs.rs/rvoip-sip/{version}/rvoip_sip/{rustdoc_path}",
                    text,
                )

    def test_missing_local_path_and_anchor_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "0.3.10"\n',
                encoding="utf-8",
            )
            (root / "crates/rvoip").mkdir(parents=True)
            (root / "README.md").write_text(
                "# Title\n[bad](missing.md) [anchor](#absent)\n",
                encoding="utf-8",
            )
            (root / "crates/rvoip/README.md").write_text("# Facade\n", encoding="utf-8")
            errors = readme_links.validate_local_links(root)
            self.assertTrue(any("missing path" in error for error in errors))
            self.assertTrue(any("missing anchor" in error for error in errors))

    def test_stale_release_version_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "0.3.10"\n',
                encoding="utf-8",
            )
            (root / "crates/rvoip").mkdir(parents=True)
            required = " ".join(
                (
                    "https://crates.io/crates/rvoip/0.3.10",
                    "https://docs.rs/rvoip/0.3.10/rvoip/",
                    "https://docs.rs/rvoip-sip/0.3.10/rvoip_sip/",
                    "https://img.shields.io/docsrs/rvoip/0.3.10",
                    "https://img.shields.io/crates/v/rvoip.svg?release=0.3.10",
                    "https://img.shields.io/crates/v/rvoip-sip.svg?release=0.3.10",
                    "[stale](https://docs.rs/rvoip/0.3.9/rvoip/)",
                )
            )
            (root / "README.md").write_text(required, encoding="utf-8")
            (root / "crates/rvoip/README.md").write_text("# Facade\n", encoding="utf-8")
            errors = readme_links.validate_release_surfaces(root)
            self.assertTrue(any("stale release versions: 0.3.9" in error for error in errors))

    def test_stale_canonical_release_evidence_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            docs = root / "crates/sip/rvoip-sip/docs"
            docs.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "0.3.10"\n',
                encoding="utf-8",
            )
            (docs / "BETA_RELEASE_REPORT.md").write_text(
                "# RVoIP 0.3.9 Release Qualification Report\n", encoding="utf-8"
            )
            (docs / "BETA_PERFORMANCE_REPORT.md").write_text(
                "Release: `0.3.9`\n", encoding="utf-8"
            )
            (docs / "BETA_RELEASE_CHECKLIST.md").write_text(
                "Current published qualified runtime crate version: `0.3.9`.\n",
                encoding="utf-8",
            )
            for filename in (
                "QUALIFICATION_SUMMARY.json",
                "QUALIFICATION_REPORT_ATTESTATION.json",
            ):
                (docs / filename).write_text(
                    '{"release":{"version":"0.3.9"}}\n', encoding="utf-8"
                )
            history = docs / "releases/qualification"
            history.mkdir(parents=True)
            (history / "README.md").write_text("# History\n", encoding="utf-8")
            errors = readme_links.validate_current_release_evidence(root)
            self.assertTrue(any("canonical release report" in error for error in errors))
            self.assertTrue(any("canonical performance report" in error for error in errors))
            self.assertTrue(any("release checklist" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
