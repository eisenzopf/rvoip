#!/usr/bin/env python3
"""Verify that the reviewed Jambonz OSS pins still name latest upstream."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import urllib.request


ROOT = Path(__file__).resolve().parent
REPOSITORIES = {
    "inbound": "jambonz/sbc-inbound",
    "outbound": "jambonz/sbc-outbound",
}


def read_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key or not value:
            raise RuntimeError(f"invalid pin line: {raw!r}")
        values[key] = value
    return values


def upstream_head(repository: str) -> str:
    completed = subprocess.run(
        ["git", "ls-remote", f"https://github.com/{repository}.git", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    )
    sha = completed.stdout.split()[0]
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise RuntimeError(f"invalid upstream revision for {repository}: {sha!r}")
    return sha


def upstream_version(repository: str, revision: str) -> str:
    url = f"https://raw.githubusercontent.com/{repository}/{revision}/package.json"
    request = urllib.request.Request(url, headers={"User-Agent": "rvoip-jambonz-interop/1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return str(json.load(response)["version"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pins", type=Path, default=ROOT / "versions.env")
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    pins = read_env(args.pins)
    expected_line = pins["JAMBONZ_RELEASE_LINE"]
    receipt: dict[str, object] = {
        "schema": "rvoip-jambonz-latest-check-v1",
        "selected_release_line": expected_line,
        "components": {},
    }
    failures: list[str] = []
    components = receipt["components"]
    assert isinstance(components, dict)
    for name, repository in REPOSITORIES.items():
        pin_key = f"JAMBONZ_{name.upper()}_COMMIT"
        pinned = pins[pin_key]
        head = upstream_head(repository)
        version = upstream_version(repository, head)
        components[name] = {
            "repository": repository,
            "pinned_revision": pinned,
            "upstream_head": head,
            "upstream_version": version,
        }
        if head != pinned:
            failures.append(f"{repository} moved from {pinned} to {head}")
        if version != expected_line:
            failures.append(
                f"{repository} reports {version}, expected release line {expected_line}"
            )
    receipt["status"] = "FAIL" if failures else "PASS"
    receipt["failures"] = failures
    rendered = json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    if args.receipt:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    if failures:
        raise SystemExit("; ".join(failures))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
