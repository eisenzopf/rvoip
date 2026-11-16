#!/usr/bin/env python3
"""Tests for the pinned Webex BYOVA protobuf fetch (no network)."""

from __future__ import annotations

import hashlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fetch_webex_byova_protos as fetch  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def fake_root(directory: Path, contents: dict[str, bytes]) -> Path:
    pins = directory / fetch.PINS
    pins.parent.mkdir(parents=True)
    pins.write_text(
        "".join(
            f"{hashlib.sha256(data).hexdigest()}  {name}\n" for name, data in contents.items()
        )
    )
    return directory


class FetchTests(unittest.TestCase):
    def test_pins_cover_exactly_the_contract_and_no_proto_is_tracked(self) -> None:
        self.assertEqual(set(fetch.pins(ROOT)), set(fetch.FILES))
        listed = subprocess.run(
            ["git", "ls-files", "*.proto"], cwd=ROOT, text=True, capture_output=True, check=False
        )
        if listed.returncode != 0:
            self.skipTest("not a git checkout")
        tracked = [path for path in listed.stdout.split() if Path(path).name in fetch.FILES]
        self.assertEqual(tracked, [], "Cisco's protobuf files must not be committed")

    def test_fetch_writes_verified_files_and_reuses_them_offline(self) -> None:
        contents = {name: f"contract {name}".encode() for name in fetch.FILES}
        calls: list[str] = []

        def serve(url: str) -> bytes:
            calls.append(url)
            return contents[url.rsplit("/", 1)[1]]

        with tempfile.TemporaryDirectory() as directory:
            root = fake_root(Path(directory), contents)
            destination = fetch.ensure(root, fetch=serve)
            self.assertEqual(destination, root / fetch.DESTINATION)
            for name, data in contents.items():
                self.assertEqual((destination / name).read_bytes(), data)
            self.assertTrue(all(url.startswith(fetch.SOURCE) for url in calls))
            self.assertIn(fetch.COMMIT, fetch.SOURCE)

            def offline(url: str) -> bytes:
                raise AssertionError(f"unexpected download of {url}")

            fetch.ensure(root, fetch=offline)

    def test_mismatched_download_fails_closed_without_writing(self) -> None:
        contents = {name: b"expected" + name.encode() for name in fetch.FILES}
        with tempfile.TemporaryDirectory() as directory:
            root = fake_root(Path(directory), contents)
            with self.assertRaises(fetch.FetchError):
                fetch.ensure(root, fetch=lambda url: b"tampered")
            destination = root / fetch.DESTINATION
            self.assertEqual(sorted(p.name for p in destination.iterdir()), [])

    def test_only_builds_of_the_grpc_feature_need_the_contract(self) -> None:
        needed = [
            "env RUSTDOCFLAGS=-D warnings cargo doc --locked --workspace --all-features --no-deps",
            "cargo clippy --workspace --all-targets --all-features",
            "cargo test -p rvoip-webex-byova --all-features",
            "cargo test -p rvoip-webex-byova --features grpc --test grpc",
            "cargo check -p rvoip --features rvoip-webex-byova/grpc",
        ]
        not_needed = [
            "cargo test --locked --workspace --exclude rvoip-sip --lib",
            "cargo test -p rvoip-core --all-features --all-targets",
            "cargo test -p rvoip-webex-byova --features wav-compat",
            "cargo test --workspace --exclude rvoip-webex-byova --all-features",
            "python3 scripts/ci/run_checks.py shard --packages rvoip-webex-byova",
        ]
        for command in needed:
            self.assertTrue(fetch.needed(command.split()), command)
        for command in not_needed:
            self.assertFalse(fetch.needed(command.split()), command)


if __name__ == "__main__":
    unittest.main()
