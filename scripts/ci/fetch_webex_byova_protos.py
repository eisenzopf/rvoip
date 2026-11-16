#!/usr/bin/env python3
"""Fetch Cisco's Webex BYOVA protobuf contract for `rvoip-webex-byova/grpc`.

Cisco publishes `voicevirtualagent.proto` and `byova_common.proto` without a
licence, so rvoip neither packages nor commits them. Builds that enable the
`rvoip-webex-byova` crate's `grpc` feature (every `--all-features` workspace
build does) read them from `target/third_party/cisco-webex-byova-proto/`, or
from `RVOIP_WEBEX_BYOVA_PROTO_DIR`.

This script downloads both files from webex/dataSourceSchemas at the pinned
commit into that ignored directory and verifies them against the SHA-256 pins
in `third_party/cisco-webex-byova-proto/SHA256SUMS`. It fails closed: a file
whose digest does not match is never written, and the script exits non-zero.
Files already present with the pinned digest are not downloaded again, so the
script is cheap to run before every build and works offline once fetched.

Usage: python3 scripts/ci/fetch_webex_byova_protos.py [--dest DIR]
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
import time
import urllib.request
from pathlib import Path
from typing import Callable

COMMIT = "fca090dfa806e28695acb145e927ad0d557ce7fe"
SOURCE = (
    "https://raw.githubusercontent.com/webex/dataSourceSchemas/"
    f"{COMMIT}/Services/VoiceVirtualAgent/"
    "5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto"
)
FILES = ("voicevirtualagent.proto", "byova_common.proto")
PINS = Path("third_party/cisco-webex-byova-proto/SHA256SUMS")
DESTINATION = Path("target/third_party/cisco-webex-byova-proto")
PACKAGE = "rvoip-webex-byova"
ENV = "RVOIP_WEBEX_BYOVA_PROTO_DIR"
MAX_BYTES = 1024 * 1024


class FetchError(RuntimeError):
    """The pinned contract could not be obtained intact."""


def root_dir() -> Path:
    return Path(__file__).resolve().parents[2]


def pins(root: Path) -> dict[str, str]:
    expected: dict[str, str] = {}
    for line in (root / PINS).read_text().splitlines():
        if not line.strip():
            continue
        digest, name = line.split(maxsplit=1)
        expected[name.lstrip("*")] = digest.lower()
    if set(expected) != set(FILES) or any(len(d) != 64 for d in expected.values()):
        raise FetchError(f"{PINS} must pin exactly {', '.join(FILES)}")
    return expected


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def download(url: str) -> bytes:
    last: Exception | None = None
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=30) as response:
                data = response.read(MAX_BYTES + 1)
            if len(data) > MAX_BYTES:
                raise FetchError(f"{url} exceeds {MAX_BYTES} bytes")
            return data
        except (OSError, ValueError) as error:
            last = error
            time.sleep(2 * (attempt + 1))
    raise FetchError(f"cannot download {url}: {last}")


def ensure(
    root: Path,
    destination: Path | None = None,
    fetch: Callable[[str], bytes] = download,
) -> Path:
    """Make the pinned files present and verified in `destination`."""
    expected = pins(root)
    destination = destination or root / DESTINATION
    destination.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        path = destination / name
        if path.is_file() and digest(path.read_bytes()) == expected[name]:
            continue
        data = fetch(f"{SOURCE}/{name}")
        actual = digest(data)
        if actual != expected[name]:
            raise FetchError(
                f"{name}: SHA-256 {actual} does not match the pinned {expected[name]}"
            )
        partial = path.with_suffix(path.suffix + ".partial")
        partial.write_bytes(data)
        os.replace(partial, path)
    return destination


def needed(argv: list[str]) -> bool:
    """Whether a command builds `rvoip-webex-byova` with its `grpc` feature."""
    if "cargo" not in argv:
        return False
    if any(f"{PACKAGE}/grpc" in value for value in argv):
        return True
    pairs = list(zip(argv, argv[1:]))
    explicit = any(flag in ("-p", "--package") and value == PACKAGE for flag, value in pairs)
    excluded = any(flag == "--exclude" and value == PACKAGE for flag, value in pairs)
    workspace = ("--workspace" in argv or "--all" in argv) and not excluded
    if "--all-features" in argv:
        return explicit or workspace
    for flag, value in pairs:
        features = value if flag in ("--features", "-F") else None
        if flag.startswith("--features="):
            features = flag.split("=", 1)[1]
        if features and "grpc" in features.replace(",", " ").split():
            return explicit
    return False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dest", type=Path, help=f"default: {DESTINATION}")
    args = parser.parse_args(argv)
    root = root_dir()
    try:
        destination = ensure(root, args.dest)
    except (FetchError, OSError) as error:
        print(f"fetch_webex_byova_protos: {error}", file=sys.stderr)
        return 1
    print(destination)
    if os.environ.get(ENV) and Path(os.environ[ENV]) != destination:
        print(f"note: {ENV} is set and takes precedence over {destination}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
