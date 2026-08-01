#!/usr/bin/env python3
"""Reject alternate async runtimes from the all-feature workspace graph."""

from __future__ import annotations

import re
import subprocess
import sys


FORBIDDEN = re.compile(
    r"^(smol|async-std|async-io|async-global-executor|async-executor|"
    r"async-channel|async-task|async-process|async-net|async-fs|async-signal|"
    r"async-broadcast|blocking|futures-lite|piper|polling) v"
)


def main() -> int:
    completed = subprocess.run(
        [
            "cargo",
            "tree",
            "--workspace",
            "--all-features",
            "--locked",
            "--prefix",
            "none",
        ],
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        sys.stdout.write(completed.stdout)
        sys.stderr.write(completed.stderr)
        return completed.returncode
    forbidden = sorted({line for line in completed.stdout.splitlines() if FORBIDDEN.match(line)})
    if forbidden:
        print("alternate runtime dependencies are forbidden:", file=sys.stderr)
        print("\n".join(forbidden), file=sys.stderr)
        return 1
    print("Tokio is the only async runtime in the all-feature workspace graph.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
