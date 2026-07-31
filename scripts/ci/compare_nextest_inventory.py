#!/usr/bin/env python3
"""Fail unless cargo test and nextest discover the same non-doctest count."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any


def cargo_count(path: Path) -> int:
    return sum(
        1
        for line in path.read_text(errors="replace").splitlines()
        if line.rstrip().endswith((": test", ": benchmark"))
    )


def nextest_count(value: Any) -> int:
    if isinstance(value, dict):
        count = 0
        for key, child in value.items():
            if key == "testcases" and isinstance(child, dict):
                count += len(child)
            else:
                count += nextest_count(child)
        return count
    if isinstance(value, list):
        return sum(nextest_count(child) for child in value)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", type=Path, required=True)
    parser.add_argument("--nextest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    cargo = cargo_count(args.cargo)
    payload = json.loads(args.nextest.read_text())
    nextest = nextest_count(payload)
    result = {
        "schema": "rvoip-nextest-parity-v1",
        "cargo_test_count": cargo,
        "nextest_count": nextest,
        "status": "PASS" if cargo == nextest and cargo > 0 else "FAIL",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
