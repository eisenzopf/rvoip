#!/usr/bin/env python3
"""Build deterministic, duration-balanced rvoip-sip integration-test partitions."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
from typing import Any


TARGET = re.compile(r"^[A-Za-z0-9_-]+$")

# Estimates are deliberately coarse. They only need to keep the known long
# process/network fixtures from accumulating on one worker. Unknown tests get a
# small non-zero weight so new targets are distributed instead of ignored.
ESTIMATED_SECONDS = {
    "audio_roundtrip_integration": 630,
    "endpoint_unified_auth": 90,
    "blind_transfer_integration": 55,
    "bridge_roundtrip_integration": 35,
    "endpoint_audio_roundtrip_integration": 20,
}
DEFAULT_SECONDS = 5


class PartitionError(RuntimeError):
    """Cargo metadata cannot be partitioned safely."""


def eligible_targets(metadata: dict[str, Any], package_name: str) -> list[str]:
    packages = [item for item in metadata.get("packages", []) if item.get("name") == package_name]
    if len(packages) != 1:
        raise PartitionError(f"expected exactly one {package_name!r} package")
    targets = []
    for target in packages[0].get("targets", []):
        if "test" not in target.get("kind", []):
            continue
        # `cargo test --tests` skips targets whose required features are not
        # enabled. Keep the split command exactly equivalent.
        if target.get("required-features", []):
            continue
        name = target.get("name", "")
        if not TARGET.fullmatch(name):
            raise PartitionError(f"unsafe test target name: {name!r}")
        targets.append(name)
    if not targets or len(targets) != len(set(targets)):
        raise PartitionError("integration target inventory is empty or contains duplicates")
    return sorted(targets)


def partition_targets(targets: list[str], count: int) -> list[dict[str, Any]]:
    if count < 1 or count > len(targets):
        raise PartitionError("partition count must be between one and the target count")
    bins: list[dict[str, Any]] = [
        {"targets": [], "estimated_seconds": 0} for _ in range(count)
    ]
    weighted = sorted(
        ((ESTIMATED_SECONDS.get(name, DEFAULT_SECONDS), name) for name in targets),
        key=lambda item: (-item[0], item[1]),
    )
    for weight, name in weighted:
        destination = min(
            range(count),
            key=lambda index: (bins[index]["estimated_seconds"], index),
        )
        bins[destination]["targets"].append(name)
        bins[destination]["estimated_seconds"] += weight
    for item in bins:
        item["targets"].sort()
    return bins


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--partitions", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    completed = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        raise PartitionError(completed.stderr.strip() or "cargo metadata failed")
    targets = eligible_targets(json.loads(completed.stdout), "rvoip-sip")
    payload = {
        "schema": "rvoip-sip-test-partitions-v1",
        "package": "rvoip-sip",
        "target_count": len(targets),
        "partitions": partition_targets(targets, args.partitions),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
